"""Tests for the deterministic extraction core. Pure stdlib — run without installing the
web deps:  python3 -m pytest services/extractor/test_extractor.py   (or run this file)."""

import extractor

MICRO_CENTER = """Micro Center
Order # MC-10293
2026-03-02 14:31
1 x GeForce RTX 4090  SKU:GPU123  SN:GPU-2291X  $1599.00  $1599.00
2 x HDMI Cable  SKU:CAB9  $9.99  $19.98
Subtotal $1618.98
Tax $133.57
Total $1752.55
"""


def test_known_vendor_template_path():
    r = extractor.extract(MICRO_CENTER)
    assert r["engine"] == "template"
    assert r["vendor"] == "Micro Center"
    assert r["order_number"] == "MC-10293"
    assert r["purchase_datetime"] == "2026-03-02T14:31"
    assert len(r["line_items"]) == 2
    gpu = r["line_items"][0]
    assert gpu["quantity"] == 1
    assert gpu["serials"] == ["GPU-2291X"]
    assert gpu["unit_price"] == 1599.00
    assert r["total"] == 1752.55


def test_partial_serials_flagged():
    text = "Micro Center\n2 x GPU  SN:ONLYONE  $10.00  $20.00\n"
    r = extractor.extract(text)
    assert r["line_items"][0]["partial_serials"] is True


def test_unknown_vendor_falls_back_to_vlm_stub():
    r = extractor.extract("Some random corner store receipt with no parseable lines")
    assert r["engine"] == "vlm_stub"
    assert r["line_items"] == []
    assert "note" in r


def test_vendor_hint_used_when_not_detected():
    text = "1 x Widget  $5.00  $5.00\n"
    r = extractor.extract(text, vendor_hint="Newegg")
    assert r["vendor"] == "Newegg"
    assert r["engine"] == "template"
    assert len(r["line_items"]) == 1


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print(f"ok - {name}")
    print("all extractor tests passed")
