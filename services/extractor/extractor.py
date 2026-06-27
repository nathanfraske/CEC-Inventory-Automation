"""Receipt extraction core (scope §11). Pure stdlib so it is unit-testable without the web
layer or any ML runtime. Two paths, one output schema (scope §11.4):

  * template fast-path  — deterministic regex parsers for known vendors (no model, no GPU)
  * VLM fallback        — a stub here; the inference box wires a real vision-language model

The web layer (app.py) wraps these; the Rust backend POSTs text/images and receives this
JSON. Adding a real vendor template = adding an entry to TEMPLATES.
"""

from __future__ import annotations

import re
from typing import Optional

# ---- output schema helpers ----------------------------------------------------

def _empty_result(vendor: Optional[str], engine: str) -> dict:
    return {
        "vendor": vendor,
        "purchase_datetime": None,
        "order_number": None,
        "invoice_number": None,
        "currency": "USD",
        "line_items": [],
        "shipments": [],
        "subtotal": None,
        "tax": None,
        "shipping": None,
        "discount_total": None,
        "total": None,
        "field_confidence": {"vendor": 0.0, "total": 0.0, "datetime": 0.0},
        "engine": engine,
    }


def _money(s: str) -> float:
    return round(float(s.replace(",", "").replace("$", "")), 2)


# ---- template fast-path -------------------------------------------------------

# A line item: "<qty> x <description>  [SKU:<sku>]  [SN:<serial>]  $<unit>  $<line>"
_LINE_RE = re.compile(
    r"^\s*(?P<qty>\d+)\s*x?\s+(?P<desc>.+?)"
    r"(?:\s+SKU:(?P<sku>\S+))?"
    r"(?:\s+SN:(?P<serial>\S+))?"
    r"\s+\$(?P<unit>[\d,]+\.\d{2})\s+\$(?P<line>[\d,]+\.\d{2})\s*$",
    re.M,
)
_ORDER_RE = re.compile(r"order\s*#\s*(?P<num>\S+)", re.I)
_INVOICE_RE = re.compile(r"invoice\s*#\s*(?P<num>\S+)", re.I)
_TOTAL_RE = re.compile(r"^\s*total\s+\$(?P<v>[\d,]+\.\d{2})", re.I | re.M)
_SUBTOTAL_RE = re.compile(r"^\s*subtotal\s+\$(?P<v>[\d,]+\.\d{2})", re.I | re.M)
_TAX_RE = re.compile(r"^\s*tax\s+\$(?P<v>[\d,]+\.\d{2})", re.I | re.M)
_SHIP_RE = re.compile(r"^\s*shipping\s+\$(?P<v>[\d,]+\.\d{2})", re.I | re.M)
_DATE_RE = re.compile(r"(\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}(?::\d{2})?)?)")

# Vendors whose receipts we recognize. Order matters only for first match.
KNOWN_VENDORS = ["Micro Center", "Newegg", "Amazon", "Mouser", "DigiKey", "LCSC"]

# Vendors that print serials per line (template can pull them reliably, scope §2/§11.1).
SERIAL_VENDORS = {"Micro Center"}


def detect_vendor(text: str, vendor_hint: Optional[str]) -> Optional[str]:
    if vendor_hint:
        return vendor_hint
    low = text.lower()
    for v in KNOWN_VENDORS:
        if v.lower() in low:
            return v
    return None


def _parse_line(m: re.Match, vendor: Optional[str]) -> dict:
    qty = int(m.group("qty"))
    serial = m.group("serial")
    serials = [serial] if serial else []
    # A serialized vendor with one printed serial for a multi-qty line → partial (flagged).
    partial = bool(serials) and qty > len(serials)
    return {
        "description": m.group("desc").strip(),
        "vendor_sku": m.group("sku"),
        "quantity": qty,
        "unit_price": _money(m.group("unit")),
        "line_total": _money(m.group("line")),
        "serials": serials,
        "is_bundle": False,
        "partial_serials": partial,
        "confidence": 0.95,
    }


def template_extract(text: str, vendor: str) -> dict:
    result = _empty_result(vendor, "template")
    for m in _LINE_RE.finditer(text):
        result["line_items"].append(_parse_line(m, vendor))

    if (m := _ORDER_RE.search(text)):
        result["order_number"] = m.group("num")
    if (m := _INVOICE_RE.search(text)):
        result["invoice_number"] = m.group("num")
    if (m := _DATE_RE.search(text)):
        dt = m.group(1).replace(" ", "T")
        result["purchase_datetime"] = dt if "T" in dt else dt + "T00:00:00"
        result["field_confidence"]["datetime"] = 0.9
    for key, rx in (("subtotal", _SUBTOTAL_RE), ("tax", _TAX_RE),
                    ("shipping", _SHIP_RE), ("total", _TOTAL_RE)):
        if (m := rx.search(text)):
            result[key] = _money(m.group("v"))
    result["field_confidence"]["vendor"] = 1.0
    result["field_confidence"]["total"] = 0.9 if result["total"] is not None else 0.0
    return result


def vlm_extract_stub(text: str, vendor: Optional[str]) -> dict:
    """Fallback for arbitrary/first-seen receipts. The real implementation runs a VLM
    (Qwen2.5-VL etc., scope §11.2) on the inference box; here it returns a low-confidence
    empty structure so the pipeline degrades instead of failing (scope §10.4)."""
    result = _empty_result(vendor, "vlm_stub")
    result["note"] = "VLM not available in this build; wire a model on the inference box (scope §11.2)"
    return result


def extract(text: str, vendor_hint: Optional[str] = None) -> dict:
    """Route to the deterministic template when the vendor is known and it parses at least
    one line; otherwise fall back to the VLM path (stubbed here)."""
    vendor = detect_vendor(text, vendor_hint)
    if vendor:
        templated = template_extract(text, vendor)
        if templated["line_items"]:
            return templated
    return vlm_extract_stub(text, vendor)
