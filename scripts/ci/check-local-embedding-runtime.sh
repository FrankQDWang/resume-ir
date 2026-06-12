#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "local embedding runtime check missing expected text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  label="$3"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "local embedding runtime check leaked $label"
  fi
}

runtime="scripts/local/embedding-runtime-sentence-transformers.py"
if [ ! -f "$runtime" ]; then
  fail "missing local embedding runtime adapter"
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-local-embedding-runtime.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$tmpdir/sentence_transformers"
cat >"$tmpdir/sentence_transformers/__init__.py" <<'PY'
class SentenceTransformer:
    def __init__(self, model_name_or_path, cache_folder=None, local_files_only=True):
        if not local_files_only:
            raise RuntimeError("adapter must not allow runtime downloads by default")
        self.model_name_or_path = model_name_or_path
        self.cache_folder = cache_folder

    def encode(self, texts, normalize_embeddings=True, show_progress_bar=False):
        return [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
        ][: len(texts)]
PY

input_file="$tmpdir/PRIVATE-embedding-input.txt"
cat >"$input_file" <<'EOF'
resume-ir-embedding-input-v1
model_id=sentence-transformers/all-MiniLM-L6-v2
dimension=4
count=2
input=doc-private-1	28
text:
PRIVATE raw resume text 1
--resume-ir-embedding-input-boundary--
input=doc-private-2	28
text:
PRIVATE raw resume text 2
--resume-ir-embedding-input-boundary--
EOF

stdout_file="$tmpdir/stdout.txt"
stderr_file="$tmpdir/stderr.txt"
PYTHONPATH="$tmpdir" \
RESUME_IR_EMBEDDING_INPUT_PATH="$input_file" \
RESUME_IR_EMBEDDING_MODEL_ID="sentence-transformers/all-MiniLM-L6-v2" \
RESUME_IR_EMBEDDING_DIMENSION=4 \
RESUME_IR_SENTENCE_TRANSFORMERS_MODEL="sentence-transformers/all-MiniLM-L6-v2" \
python3 "$runtime" >"$stdout_file" 2>"$stderr_file"

if [ -s "$stderr_file" ]; then
  fail "local embedding runtime check wrote stderr on success"
fi
require_text "$stdout_file" "resume-ir-embedding-v1"
require_text "$stdout_file" "model_id=sentence-transformers/all-MiniLM-L6-v2"
require_text "$stdout_file" "dimension=4"
require_text "$stdout_file" "vector=doc-private-1	1,0,0,0"
require_text "$stdout_file" "vector=doc-private-2	0,1,0,0"
reject_text "$stdout_file" "PRIVATE raw resume text" "raw embedding input text"
reject_text "$stdout_file" "$tmpdir" "temporary local path"
reject_text "$stdout_file" "$input_file" "input path"

printf '%s\n' "local embedding runtime check passed"
