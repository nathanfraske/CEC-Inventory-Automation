# Extractor + stitching service (scope §11)

Python (FastAPI) service that turns a receipt into structured line items (scope §11.4),
running on the inference box, not the web host. Two paths, one schema:

- **Template fast-path** (`extractor.template_extract`) — deterministic regex parsers for
  known vendors (Micro Center, Newegg, …). No model, no GPU, no hallucination. Vendors that
  print serials (Micro Center) are pulled per line. Used for pasted/OCR'd **text**.
- **Image vision** (`vision.extract_image`) — for a receipt **image** (first-seen receipts
  where only a photo exists). Backend via `EXTRACTOR_VLM_BACKEND`:
  - `stub` (default) — empty, clearly-noted result; keeps the build/CI hermetic.
  - `claude` — the **interim** path: POSTs the image to the Anthropic Messages API (a Claude
    vision model) and parses the §11.4 JSON. Needs `ANTHROPIC_API_KEY` + `EXTRACTOR_VLM_MODEL`
    in the gitignored `.env`. Privacy: this sends the image to a third-party API — opt-in,
    off by default; swap for the local VLM (Qwen2.5-VL etc., scope §11.2) on the inference box.

`extractor.py` and `vision.py` are pure stdlib so both are unit-testable without the web deps
or any ML runtime — `vision`'s network call is an injectable `_transport`.

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
