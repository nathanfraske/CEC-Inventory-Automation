# Extractor web layer (scope §11.3). The vision model + OpenCV are added on the inference
# box image; this base runs the deterministic template fast-path and the API.
FROM python:3.14-slim
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY extractor.py vision.py app.py ./
# Run as a non-root user.
RUN useradd -r -u 10001 cec && chown -R cec:cec /app
USER cec
EXPOSE 8900
CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8900"]
