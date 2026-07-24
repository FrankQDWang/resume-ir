#!/bin/sh
set -eu

set -- "$@" \
  --embedding-command "$RESUME_IR_TEST_EMBEDDING_COMMAND" \
  --embedding-model-id "$RESUME_IR_TEST_EMBEDDING_MODEL_ID" \
  --embedding-dimension "$RESUME_IR_TEST_EMBEDDING_DIMENSION" \
  --embedding-timeout-ms 15000

if [ -n "${RESUME_IR_TEST_CLASSIFIER_MODEL:-}" ]; then
  set -- "$@" --resume-classifier-model "$RESUME_IR_TEST_CLASSIFIER_MODEL"
fi

if [ -n "${RESUME_IR_TEST_OCR_COMMAND:-}" ]; then
  set -- "$@" \
    --ocr-tesseract-command "$RESUME_IR_TEST_OCR_COMMAND" \
    --ocr-render-command "$RESUME_IR_TEST_OCR_RENDER_COMMAND" \
    --ocr-lang eng+chi_sim
fi

exec "$RESUME_IR_TEST_DAEMON_BIN" "$@"
