"""Tests for the vision backend's parse/normalize path. Hermetic: the network call is an
injected transport, so no API key or outbound request is needed. Run:
    python3 -m pytest services/extractor/test_vision.py   (or run this file)."""

import json

import vision

# A representative model reply (what the Anthropic backend would return as text), including a
# stray markdown fence + per-line serials + string-typed quantity, to exercise normalization.
_MODEL_REPLY = """```json
{
  "vendor": "Newegg",
  "purchase_datetime": "2026-04-01T10:00:00",
  "order_number": "NE-77",
  "currency": "USD",
  "line_items": [
    {"description": "RTX 4080", "vendor_sku": "GPU8", "quantity": "1",
     "unit_price": 1099.0, "line_total": 1099.0, "serials": ["SN-A", ""], "is_bundle": false},
    {"description": "DDR5 32GB", "quantity": 2, "unit_price": 120.0, "line_total": 240.0}
  ],
  "subtotal": 1339.0, "tax": 110.0, "total": 1449.0
}
```"""


def _fake_transport(image_b64, media_type):
    assert image_b64, "image should be base64-encoded before transport"
    assert media_type == "image/png"
    return _MODEL_REPLY


def test_vision_parses_and_normalizes_model_reply():
    r = vision.extract_image(
        b"\x89PNG fake bytes", media_type="image/png", _transport=_fake_transport
    )
    assert r["engine"] == "vlm_claude"
    assert r["vendor"] == "Newegg"
    assert r["order_number"] == "NE-77"
    assert r["total"] == 1449.0
    assert len(r["line_items"]) == 2
    gpu = r["line_items"][0]
    assert gpu["quantity"] == 1  # coerced from the string "1"
    assert gpu["serials"] == ["SN-A"]  # the empty serial is dropped
    ram = r["line_items"][1]
    assert ram["quantity"] == 2
    assert ram["serials"] == []
    # canonical schema keys are always present even when the model omitted them
    assert "discount_total" in r and r["discount_total"] is None
    assert r["field_confidence"]["total"] == 0.6


def test_vision_tolerates_prose_wrapped_json():
    reply = 'Here is the receipt data:\n{"vendor":"Mouser","line_items":[],"total":5.0}\nThanks!'
    r = vision.extract_image(b"x", _transport=lambda b, m: reply)
    assert r["vendor"] == "Mouser"
    assert r["total"] == 5.0
    assert r["line_items"] == []


def test_vision_vendor_hint_used_when_model_omits_vendor():
    reply = json.dumps({"line_items": [], "total": None})
    r = vision.extract_image(b"x", vendor_hint="LCSC", _transport=lambda b, m: reply)
    assert r["vendor"] == "LCSC"
    assert r["field_confidence"]["vendor"] == 0.6


def test_vision_stub_default_is_hermetic():
    # No transport + default backend (stub) → empty, clearly-noted result, no network.
    r = vision.extract_image(b"x")
    assert r["engine"] == "vlm_stub"
    assert r["line_items"] == []
    assert "note" in r


def test_vision_openai_backend_tags_engine():
    # backend=openai with an injected transport → result is tagged vlm_openai (not vlm_claude).
    import os

    prev = os.environ.get("EXTRACTOR_VLM_BACKEND")
    os.environ["EXTRACTOR_VLM_BACKEND"] = "openai"
    try:
        r = vision.extract_image(b"x", _transport=lambda b, m: '{"line_items": [], "total": 1.0}')
        assert r["engine"] == "vlm_openai"
        assert r["total"] == 1.0
    finally:
        if prev is None:
            os.environ.pop("EXTRACTOR_VLM_BACKEND", None)
        else:
            os.environ["EXTRACTOR_VLM_BACKEND"] = prev


def test_openai_transport_builds_openai_chat_request():
    # Lock the OpenAI/broker wire format without a network call: capture the urllib Request.
    import os
    import urllib.request

    captured = {}

    class _FakeResp:
        def __init__(self, body):
            self._body = body

        def read(self):
            return self._body

        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

    def _fake_urlopen(req, timeout=None):
        captured["url"] = req.full_url
        captured["headers"] = {k.lower(): v for k, v in req.header_items()}
        captured["body"] = json.loads(req.data.decode("utf-8"))
        return _FakeResp(
            json.dumps(
                {"choices": [{"message": {"content": '{"vendor":"Newegg","line_items":[],"total":3.0}'}}]}
            ).encode("utf-8")
        )

    keys = ("EXTRACTOR_VLM_BASE_URL", "EXTRACTOR_VLM_MODEL", "EXTRACTOR_VLM_API_KEY")
    prev = {k: os.environ.get(k) for k in keys}
    real_urlopen = urllib.request.urlopen
    os.environ["EXTRACTOR_VLM_BASE_URL"] = "http://broker:8080/v1"
    os.environ["EXTRACTOR_VLM_MODEL"] = "cec-worker-vision"
    os.environ["EXTRACTOR_VLM_API_KEY"] = "tok123"
    urllib.request.urlopen = _fake_urlopen
    try:
        text = vision._openai_transport("BASE64DATA", "image/png")
    finally:
        urllib.request.urlopen = real_urlopen
        for k, v in prev.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    assert captured["url"].endswith("/v1/chat/completions")
    body = captured["body"]
    assert body["model"] == "cec-worker-vision"
    content = body["messages"][0]["content"]
    assert content[0]["type"] == "text"
    assert content[1]["type"] == "image_url"
    assert content[1]["image_url"]["url"] == "data:image/png;base64,BASE64DATA"
    assert captured["headers"]["authorization"] == "Bearer tok123"
    assert '"vendor":"Newegg"' in text  # transport returns the model's text for the parser


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print(f"ok - {name}")
    print("all vision tests passed")
