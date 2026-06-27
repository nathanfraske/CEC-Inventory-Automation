"""Extractor + stitching service (scope §11.3). FastAPI on the inference box. The Rust
backend POSTs receipt text (or, later, ordered images) and receives the §11.4 JSON. Run:

    uvicorn app:app --host 0.0.0.0 --port 8900

Endpoints:
  GET  /health        liveness
  POST /extract       {text, vendor_hint?} -> structured line items (template or VLM)
  POST /stitch        placeholder for the OpenCV multi-image stitch pre-step (scope §10)
"""

from __future__ import annotations

from typing import List, Optional

from fastapi import FastAPI
from pydantic import BaseModel

import extractor

app = FastAPI(title="CEC Inventory Extractor", version="0.1.0")


class ExtractRequest(BaseModel):
    text: str
    vendor_hint: Optional[str] = None


class StitchRequest(BaseModel):
    # Object-store refs of the ordered, overlapping segments (scope §10.2).
    segment_refs: List[str] = []


@app.get("/health")
def health() -> dict:
    return {"status": "ok", "service": "extractor"}


@app.post("/extract")
def extract(req: ExtractRequest) -> dict:
    return extractor.extract(req.text, req.vendor_hint)


@app.post("/stitch")
def stitch(req: StitchRequest) -> dict:
    # The real implementation rectifies + stitches with OpenCV (screenshot row-match or ORB
    # feature stitch) and falls back to an ordered multi-page PDF on low confidence (§10.3/10.4).
    return {
        "stitched_ref": None,
        "segments": req.segment_refs,
        "note": "stitching not implemented in this build; OpenCV pre-step runs on the inference box",
    }
