# Extractor web layer (scope §11.3). The vision model + OpenCV are added on the inference
# box image; this base runs the deterministic template fast-path and the API.
FROM python:3.12-slim
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY extractor.py vision.py app.py ./
EXPOSE 8900
CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8900"]
