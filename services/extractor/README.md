# Extractor + stitching service (scope §11)

Python (FastAPI) service that turns a receipt into structured line items (scope §11.4),
running on the inference box, not the web host. Two paths, one schema:

- **Template fast-path** (`extractor.template_extract`) — deterministic regex parsers for
  known vendors (Micro Center, Newegg, …). No model, no GPU, no hallucination. Vendors that
  print serials (Micro Center) are pulled per line.
- **VLM fallback** (`extractor.vlm_extract_stub`) — a stub here; on the inference box this
  wires a vision-language model (e.g. Qwen2.5-VL, scope §11.2) for arbitrary receipts.

`extractor.py` is pure stdlib so it is unit-testable without the web deps or any ML runtime.

## Run

```sh
cp .env.example .env            # bind + model config (gitignored)
pip install -r requirements.txt
uvicorn app:app --host 0.0.0.0 --port 8900
curl -s localhost:8900/health
curl -s -X POST localhost:8900/extract -H 'content-type: application/json' \
  -d '{"text":"Micro Center\n1 x RTX 4090 SN:GPU-1 $1599.00 $1599.00\nTotal $1599.00"}'
```

## Test (no deps)

```sh
python3 -m pytest test_extractor.py      # or: python3 test_extractor.py
```

## Seam to the Rust backend

The Rust API's `extractor` client POSTs `{text, vendor_hint?}` to `EXTRACTOR_URL/extract`
and maps the result into draft `PurchaseLineItem`s for operator confirmation (scope §3).
`POST /stitch` is the placeholder for the OpenCV multi-image stitch pre-step (scope §10).
