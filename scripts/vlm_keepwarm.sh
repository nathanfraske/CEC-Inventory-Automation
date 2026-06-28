#!/usr/bin/env bash
# Keep the CEC receipt-vision model resident on the cec-llm-broker so operator scans don't pay a
# cold-load (scope §11.2 UX). Sends a minimal chat completion to reset the broker's idle reaper.
# Best-effort and idempotent; never fails the caller.
#
# Schedule it under the broker's idle_stop window via
# scripts/systemd/cec-vlm-keepwarm.{service,timer} (every ~20 min). Override the target with:
#   CEC_VLM_KEEPWARM_URL   (default http://127.0.0.1:8080/v1 — the broker, from the HOST)
#   CEC_VLM_KEEPWARM_MODEL (default cec-vision-judge)
# NOTE: the host reaches the broker at 127.0.0.1:8080; the extractor *container* uses
# host.docker.internal:8080 (see .env EXTRACTOR_VLM_BASE_URL) — different perspectives, same broker.
set -uo pipefail

# Warm the SAME model the extractor calls: prefer EXTRACTOR_VLM_MODEL from the repo .env (single
# source of truth) so the keep-warm seat can't drift from the one /extract-image actually uses;
# fall back to cec-vision-judge. Override explicitly with CEC_VLM_KEEPWARM_MODEL.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_MODEL="$(grep -m1 '^EXTRACTOR_VLM_MODEL=' "${SCRIPT_DIR}/../.env" 2>/dev/null | cut -d= -f2- | awk '{print $1}')"

URL="${CEC_VLM_KEEPWARM_URL:-http://127.0.0.1:8080/v1}"
MODEL="${CEC_VLM_KEEPWARM_MODEL:-${ENV_MODEL:-cec-vision-judge}}"

code="$(curl -s -m 180 -o /dev/null -w '%{http_code}' "${URL%/}/chat/completions" \
  -H 'content-type: application/json' \
  -H 'x-cec-client: cec-inventory-keepwarm' \
  -d "{\"model\":\"${MODEL}\",\"max_tokens\":1,\"temperature\":0,\"messages\":[{\"role\":\"user\",\"content\":\"ping\"}]}" \
  2>/dev/null || echo "000")"

echo "vlm-keepwarm: ${MODEL} @ ${URL} -> HTTP ${code}"
# Exit 0 regardless: a cold/slow broker should not spam failure mail; the next tick retries.
exit 0
