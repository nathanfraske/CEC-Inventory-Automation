"""Extractor + stitching service (scope §11.3). FastAPI on the inference box. The Rust
backend POSTs receipt text (or, later, ordered images) and receives the §11.4 JSON. Run:

    uvicorn app:app --host 0.0.0.0 --port 8900

Endpoints:
  GET  /health        liveness
  POST /extract       {text, vendor_hint?} -> structured line items (template or VLM)
  POST /extract-image {image_base64, media_type?, vendor_hint?} -> §11.4 JSON via the vision
                      backend (interim hosted-vision path; scope §11.2)
  POST /stitch        placeholder for the OpenCV multi-image stitch pre-step (scope §10)
"""

from __future__ import annotations

import base64
import os
from typing import List, Optional

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

import extractor
import vision

app = FastAPI(title="CEC Inventory Extractor", version="0.1.0")


class ExtractRequest(BaseModel):
    text: str
    vendor_hint: Optional[str] = None


class ExtractImageRequest(BaseModel):
    image_base64: str
    media_type: str = "image/jpeg"
    vendor_hint: Optional[str] = None


class StitchRequest(BaseModel):
    # Object-store refs of the ordered, overlapping segments (scope §10.2).
    segment_refs: List[str] = []


@app.get("/health")
def health() -> dict:
    # Report the active vision backend so the stack/operator can see whether the interim
    # hosted-vision path is live (stub by default).
    return {
        "status": "ok",
        "service": "extractor",
        "vlm_backend": os.environ.get("EXTRACTOR_VLM_BACKEND", "stub"),
    }


@app.post("/extract")
def extract(req: ExtractRequest) -> dict:
    return extractor.extract(req.text, req.vendor_hint)


@app.get("/vlm-status")
def vlm_status() -> dict:
    # Whether the configured vision model is warm (resident) — lets the API/UI show a
    # 'warming' vs 'ready' state before/while extracting. Best-effort; never raises.
    return vision.vlm_status()


@app.post("/extract-image")
def extract_image(req: ExtractImageRequest) -> dict:
    try:
        data = base64.b64decode(req.image_base64, validate=True)
    except ValueError:  # binascii.Error subclasses ValueError
        raise HTTPException(status_code=400, detail="image_base64 is not valid base64")
    try:
        return vision.extract_image(data, req.media_type, req.vendor_hint)
    except vision.ImageTooLargeError as e:
        raise HTTPException(status_code=413, detail=str(e))


@app.post("/stitch")
def stitch(req: StitchRequest) -> dict:
    # The real implementation rectifies + stitches with OpenCV (screenshot row-match or ORB
    # feature stitch) and falls back to an ordered multi-page PDF on low confidence (§10.3/10.4).
    return {
        "stitched_ref": None,
        "segments": req.segment_refs,
        "note": "stitching not implemented in this build; OpenCV pre-step runs on the inference box",
    }
