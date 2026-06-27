"""Vision extraction backend (scope §11.2) — the interim VLM path.

Until the local vision-language model on the inference box is wired, a receipt *image* (not
OCR'd text) can be read by a hosted vision LLM. The deterministic template fast-path
(`extractor.extract` over text) is still preferred; this is the fallback for first-seen
receipts where only an image exists.

Backends (env `EXTRACTOR_VLM_BACKEND`):
  * ``stub``   (default) — returns a low-confidence empty result; keeps CI/offline hermetic.
  * ``claude``           — POSTs the image to the Anthropic Messages API (a Claude vision
                           model) and parses the §11.4 JSON back. This is "Claude's own
                           visual ability" standing in for the local VLM.

Privacy note: the ``claude`` backend sends the receipt image to a third-party API. It is
opt-in (off by default). On the inference box, swap it for the local model and keep images
on-prem (scope §11.2). No key is ever baked in — it is read from the environment at call time.

Pure stdlib (urllib) so the deterministic build stays dependency-free. The HTTP call is an
injectable ``_transport`` so the parse/normalize path is unit-tested without network.
"""

from __future__ import annotations

import base64
import json
import os
import urllib.request
from typing import Callable, Optional

from extractor import _empty_result

ANTHROPIC_VERSION = "2023-06-01"

# The model is told to return ONLY the §11.4 JSON object so the Rust seam can map it straight
# into draft line items for operator confirmation.
_SCHEMA_PROMPT = """You are a receipt/invoice extraction engine for a computer-hardware \
inventory system. Read the attached receipt or invoice image and return ONLY a single JSON \
object (no prose, no markdown fences) with exactly these keys:
{
  "vendor": string|null,
  "purchase_datetime": ISO-8601 string|null,
  "order_number": string|null,
  "invoice_number": string|null,
  "currency": 3-letter code (default "USD"),
  "line_items": [ {
    "description": string,
    "vendor_sku": string|null,
    "quantity": integer,
    "unit_price": number|null,
    "line_total": number|null,
    "serials": [string],
    "is_bundle": boolean
  } ],
  "subtotal": number|null,
  "tax": number|null,
  "shipping": number|null,
  "discount_total": number|null,
  "total": number|null
}
Rules: money as plain numbers (no currency symbols or thousands separators); quantities as \
integers; copy any per-line serial numbers you can read into that line's "serials" array; \
use null for anything absent. Return JSON only."""


def _anthropic_transport(image_b64: str, media_type: str) -> str:
    """Call the Anthropic Messages API with the image; return the model's text output."""
    key = os.environ.get("ANTHROPIC_API_KEY")
    if not key:
        raise RuntimeError(
            "ANTHROPIC_API_KEY is required for EXTRACTOR_VLM_BACKEND=claude (set it in the "
            "gitignored .env; never commit it)"
        )
    model = os.environ.get("EXTRACTOR_VLM_MODEL")
    if not model:
        raise RuntimeError(
            "EXTRACTOR_VLM_MODEL is required for EXTRACTOR_VLM_BACKEND=claude "
            "(set it to a vision-capable model id)"
        )
    base = os.environ.get("ANTHROPIC_BASE_URL", "https://api.anthropic.com").rstrip("/")
    payload = {
        "model": model,
        "max_tokens": 2048,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": image_b64,
                        },
                    },
                    {"type": "text", "text": _SCHEMA_PROMPT},
                ],
            }
        ],
    }
    req = urllib.request.Request(
        base + "/v1/messages",
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "content-type": "application/json",
            "x-api-key": key,
            "anthropic-version": ANTHROPIC_VERSION,
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=90) as r:  # noqa: S310 (fixed host from env)
        resp = json.loads(r.read().decode("utf-8"))
    # The Messages API returns a list of content blocks; concatenate the text ones.
    return "".join(
        b.get("text", "") for b in resp.get("content", []) if b.get("type") == "text"
    )


def _parse_json(text: str) -> dict:
    """Parse the model's reply, tolerating a stray markdown fence or surrounding prose."""
    t = text.strip()
    if t.startswith("```"):
        # ```json ... ``` → keep the middle
        parts = t.split("```")
        t = parts[1] if len(parts) > 1 else t
        if t.lstrip().lower().startswith("json"):
            t = t.lstrip()[4:]
        t = t.strip()
    try:
        return json.loads(t)
    except json.JSONDecodeError:
        i, j = t.find("{"), t.rfind("}")
        if 0 <= i < j:
            return json.loads(t[i : j + 1])
        raise


def _normalize(raw: dict, vendor_hint: Optional[str]) -> dict:
    """Coerce the model's object into the canonical §11.4 schema (fill gaps, fix types)."""
    out = _empty_result(raw.get("vendor") or vendor_hint, "vlm_claude")
    for k in (
        "purchase_datetime",
        "order_number",
        "invoice_number",
        "subtotal",
        "tax",
        "shipping",
        "discount_total",
        "total",
    ):
        if raw.get(k) is not None:
            out[k] = raw[k]
    if raw.get("currency"):
        out["currency"] = raw["currency"]

    items = []
    for li in raw.get("line_items") or []:
        try:
            qty = int(li.get("quantity") or 1)
        except (TypeError, ValueError):
            qty = 1
        items.append(
            {
                "description": (li.get("description") or "").strip(),
                "vendor_sku": li.get("vendor_sku"),
                "quantity": qty,
                "unit_price": li.get("unit_price"),
                "line_total": li.get("line_total"),
                "serials": [s for s in (li.get("serials") or []) if s],
                "is_bundle": bool(li.get("is_bundle")),
                "partial_serials": False,
                "confidence": 0.6,
            }
        )
    out["line_items"] = items
    out["field_confidence"] = {
        "vendor": 0.6 if out["vendor"] else 0.0,
        "total": 0.6 if out["total"] is not None else 0.0,
        "datetime": 0.6 if out["purchase_datetime"] else 0.0,
    }
    return out


def extract_image(
    image_bytes: bytes,
    media_type: str = "image/jpeg",
    vendor_hint: Optional[str] = None,
    _transport: Optional[Callable[[str, str], str]] = None,
) -> dict:
    """Extract a receipt *image* into the §11.4 JSON. ``_transport`` is injectable for tests."""
    backend = os.environ.get("EXTRACTOR_VLM_BACKEND", "stub").lower()
    if backend == "stub" and _transport is None:
        out = _empty_result(vendor_hint, "vlm_stub")
        out["note"] = (
            "image VLM disabled (EXTRACTOR_VLM_BACKEND=stub); set it to 'claude' for the "
            "interim hosted-vision path, or wire the local VLM on the inference box (scope §11.2)"
        )
        return out
    transport = _transport or _anthropic_transport
    image_b64 = base64.b64encode(image_bytes).decode("ascii")
    raw = _parse_json(transport(image_b64, media_type))
    return _normalize(raw, vendor_hint)
