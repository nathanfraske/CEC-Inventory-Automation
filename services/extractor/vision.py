"""Vision extraction backend (scope §11.2) — the interim VLM path.

Until the local vision-language model on the inference box is wired, a receipt *image* (not
OCR'd text) can be read by a hosted vision LLM. The deterministic template fast-path
(`extractor.extract` over text) is still preferred; this is the fallback for first-seen
receipts where only an image exists.

Backends (env `EXTRACTOR_VLM_BACKEND`):
  * ``stub``   (default) — returns a low-confidence empty result; keeps CI/offline hermetic.
  * ``openai``           — POSTs the image to an OpenAI-compatible ``/chat/completions`` endpoint
                           (the on-box ``cec-llm-broker`` or any local VLM server) and parses the
                           §11.4 JSON back. Preferred on-prem path: when ``EXTRACTOR_VLM_BASE_URL``
                           points at the local broker, receipt images never leave the box (§11.2).
  * ``claude``           — POSTs the image to the Anthropic Messages API (a hosted Claude vision
                           model). Interim / off-box fallback.

Privacy note: the ``claude`` backend sends the receipt image to a third-party API; the ``openai``
backend keeps it on-box when aimed at the local broker (scope §11.2). Both are opt-in (``stub``
by default). No key is ever baked in — keys are read from the environment at call time.

Pure stdlib (urllib) so the deterministic build stays dependency-free. The HTTP call is an
injectable ``_transport`` so the parse/normalize path is unit-tested without network.
"""

from __future__ import annotations

import base64
import json
import logging
import os
import urllib.request
from typing import Callable, Optional

from extractor import _empty_result, _money_str

_log = logging.getLogger("cec.extractor.vision")

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
    "unit_price": decimal string|null,
    "line_total": decimal string|null,
    "serials": [string],
    "is_bundle": boolean
  } ],
  "subtotal": decimal string|null,
  "tax": decimal string|null,
  "shipping": decimal string|null,
  "discount_total": decimal string|null,
  "total": decimal string|null
}
Rules: money as decimal STRINGS with exactly two places (e.g. "19.99"), no currency symbols or \
thousands separators; quantities as integers; copy any per-line serial numbers you can read into \
that line's "serials" array; use null for anything absent. Return JSON only."""


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


def _openai_transport(image_b64: str, media_type: str) -> str:
    """Call an OpenAI-compatible ``/chat/completions`` endpoint (the on-box cec-llm-broker, or any
    local VLM server) with the image as a data-URI ``image_url`` block; return the model's text.

    When ``EXTRACTOR_VLM_BASE_URL`` is the local broker, the image stays on the box (scope §11.2).
    The timeout is generous so a cold model load on the broker (boot-on-demand) rides through.
    """
    base = os.environ.get("EXTRACTOR_VLM_BASE_URL")
    if not base:
        raise RuntimeError(
            "EXTRACTOR_VLM_BASE_URL is required for EXTRACTOR_VLM_BACKEND=openai "
            "(e.g. http://host.docker.internal:8080/v1 for the local cec-llm-broker)"
        )
    model = os.environ.get("EXTRACTOR_VLM_MODEL")
    if not model:
        raise RuntimeError(
            "EXTRACTOR_VLM_MODEL is required for EXTRACTOR_VLM_BACKEND=openai "
            "(a vision-capable model id, e.g. cec-worker-vision)"
        )
    base = base.rstrip("/")
    payload = {
        "model": model,
        "max_tokens": int(os.environ.get("EXTRACTOR_VLM_MAX_TOKENS", "2048")),
        "temperature": 0,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": _SCHEMA_PROMPT},
                    {
                        "type": "image_url",
                        "image_url": {"url": f"data:{media_type};base64,{image_b64}"},
                    },
                ],
            }
        ],
    }
    headers = {"content-type": "application/json", "x-cec-client": "cec-inventory-extractor"}
    key = os.environ.get("EXTRACTOR_VLM_API_KEY")
    if key:
        headers["authorization"] = f"Bearer {key}"
    req = urllib.request.Request(
        base + "/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers=headers,
        method="POST",
    )
    timeout = float(os.environ.get("EXTRACTOR_VLM_TIMEOUT", "600"))
    with urllib.request.urlopen(req, timeout=timeout) as r:  # noqa: S310 (host from env)
        resp = json.loads(r.read().decode("utf-8"))
    choices = resp.get("choices") or []
    if not choices:
        raise RuntimeError(f"vision endpoint returned no choices: {json.dumps(resp)[:300]}")
    content = (choices[0].get("message") or {}).get("content")
    # content is usually a string; some servers return a list of typed parts.
    if isinstance(content, list):
        content = "".join(p.get("text", "") for p in content if isinstance(p, dict))
    return content or ""


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


def _normalize(raw: dict, vendor_hint: Optional[str], engine: str = "vlm_claude") -> dict:
    """Coerce the model's object into the canonical §11.4 schema (fill gaps, fix types)."""
    out = _empty_result(raw.get("vendor") or vendor_hint, engine)
    money_keys = {"subtotal", "tax", "shipping", "discount_total", "total"}
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
            out[k] = _money_str(raw[k]) if k in money_keys else raw[k]
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
                "unit_price": _money_str(li.get("unit_price")),
                "line_total": _money_str(li.get("line_total")),
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


# Default transport + result-engine tag per backend.
_BACKENDS: dict[str, tuple[Callable[[str, str], str], str]] = {
    "openai": (_openai_transport, "vlm_openai"),
    "claude": (_anthropic_transport, "vlm_claude"),
}


class ImageTooLargeError(ValueError):
    """A receipt image exceeded EXTRACTOR_VLM_MAX_IMAGE_BYTES — refused before any VLM egress."""


def _egress_dest(backend: str) -> str:
    """Human-readable destination a receipt image would be sent to (for the egress audit log)."""
    if backend == "openai":
        return (os.environ.get("EXTRACTOR_VLM_BASE_URL") or "unset").rstrip("/") + "/chat/completions"
    if backend == "claude":
        base = os.environ.get("ANTHROPIC_BASE_URL", "https://api.anthropic.com").rstrip("/")
        return base + "/v1/messages"
    return backend


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
            "image VLM disabled (EXTRACTOR_VLM_BACKEND=stub); set it to 'openai' for the on-box "
            "broker/local-VLM path (scope §11.2), or 'claude' for the interim hosted path"
        )
        return out

    # Unknown backend with no injected transport falls back to the Anthropic path (historical
    # default); an injected transport keeps the backend's engine tag for test fidelity.
    default_transport, engine = _BACKENDS.get(backend, (_anthropic_transport, "vlm_claude"))
    transport = _transport or default_transport

    # Egress guard (scope §11.2 audit): cap the raw image size BEFORE it is base64-expanded and
    # shipped to the VLM endpoint, so a poisoned/oversized upload can't blow up the request or the
    # off-box payload. Generous default (16 MiB) clears legitimate phone photos.
    max_bytes = int(os.environ.get("EXTRACTOR_VLM_MAX_IMAGE_BYTES", str(16 * 1024 * 1024)))
    if len(image_bytes) > max_bytes:
        raise ImageTooLargeError(
            f"receipt image is {len(image_bytes)} bytes, over the "
            f"EXTRACTOR_VLM_MAX_IMAGE_BYTES={max_bytes} cap"
        )

    # Audit the egress (size + destination). Only for a real transport — an injected test transport
    # makes no network call. Matters most for the off-box 'claude' path (with 'openai' aimed at the
    # local broker the image stays on the box, §11.2).
    if _transport is None:
        _log.info(
            "vision egress: backend=%s model=%s dest=%s image_bytes=%d media=%s",
            backend,
            os.environ.get("EXTRACTOR_VLM_MODEL"),
            _egress_dest(backend),
            len(image_bytes),
            media_type,
        )

    image_b64 = base64.b64encode(image_bytes).decode("ascii")
    raw = _parse_json(transport(image_b64, media_type))
    return _normalize(raw, vendor_hint, engine)


def vlm_status() -> dict:
    """Report whether the configured vision model is *warm* (resident on the broker) so the UI can
    show a 'warming' vs 'ready' state before/while extracting. Best-effort — never raises.

    For ``openai`` it asks the broker's ``/models`` catalog for the model's ``running`` flag. For
    ``stub`` (instant) and ``claude`` (hosted, no local cold load) it reports warm.
    """
    backend = os.environ.get("EXTRACTOR_VLM_BACKEND", "stub").lower()
    model = os.environ.get("EXTRACTOR_VLM_MODEL") or None
    if backend != "openai":
        return {
            "backend": backend,
            "model": model,
            "warm": True,
            "detail": "no local model load for this backend",
        }
    base = os.environ.get("EXTRACTOR_VLM_BASE_URL")
    if not base or not model:
        return {
            "backend": backend,
            "model": model,
            "warm": False,
            "detail": "EXTRACTOR_VLM_BASE_URL/EXTRACTOR_VLM_MODEL not set",
        }
    try:
        req = urllib.request.Request(base.rstrip("/") + "/models", method="GET")
        with urllib.request.urlopen(req, timeout=5) as r:  # noqa: S310 (host from env)
            data = json.loads(r.read().decode("utf-8"))
        for m in data.get("data", []):
            if m.get("id") == model:
                running = bool(m.get("running"))
                return {
                    "backend": backend,
                    "model": model,
                    "warm": running,
                    "detail": "running" if running else "cold (loads on first extract)",
                }
        return {"backend": backend, "model": model, "warm": False, "detail": "model not in catalog"}
    except Exception as e:  # noqa: BLE001  (status is advisory; degrade to 'cold')
        return {
            "backend": backend,
            "model": model,
            "warm": False,
            "detail": f"broker status unavailable: {e.__class__.__name__}",
        }
