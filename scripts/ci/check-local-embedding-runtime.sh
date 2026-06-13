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
manifest_script="scripts/local/prepare-local-embedding-model-manifest.sh"
if [ ! -f "$runtime" ]; then
  fail "missing local embedding runtime adapter"
fi
if [ ! -f "$manifest_script" ]; then
  fail "missing local embedding model manifest preparation script"
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-local-embedding-runtime.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$tmpdir/sentence_transformers"
cat >"$tmpdir/sentence_transformers/__init__.py" <<'PY'
import sys

class SentenceTransformer:
    def __init__(self, model_name_or_path, cache_folder=None, local_files_only=True):
        if not local_files_only:
            raise RuntimeError("adapter must not allow runtime downloads by default")
        print("PRIVATE loader stderr must be suppressed", file=sys.stderr)
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

hf_root="$tmpdir/PRIVATE-hf-cache/hub"
snapshot="1110a243fdf4706b3f48f1d95db1a4f5529b4d41"
model_cache="$hf_root/models--sentence-transformers--all-MiniLM-L6-v2"
snapshot_dir="$model_cache/snapshots/$snapshot"
mkdir -p "$snapshot_dir" "$model_cache/refs"
printf '%s\n' "$snapshot" > "$model_cache/refs/main"
cat > "$snapshot_dir/README.md" <<'EOF'
---
license: apache-2.0
library_name: sentence-transformers
---

# Synthetic all-MiniLM model card
EOF
printf '%s\n' "SYNTHETIC MODEL WEIGHTS PLACEHOLDER" > "$snapshot_dir/model.safetensors"

fake_resume_cli="$tmpdir/fake-resume-cli"
fake_resume_cli_args="$tmpdir/fake-resume-cli-args.txt"
cat > "$fake_resume_cli" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_RESUME_CLI_ARGS"
if [ "${1:-}" != "model" ]; then
  printf 'unexpected fake resume-cli command\n' >&2
  exit 64
fi
case "${2:-}" in
  draft-manifest)
    out=""
    model_id=""
    dimension=""
    license=""
    artifact=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --out) out="$2"; shift 2 ;;
        --model-id) model_id="$2"; shift 2 ;;
        --dimension) dimension="$2"; shift 2 ;;
        --license) license="$2"; shift 2 ;;
        --artifact) artifact="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    [ -n "$out" ] || exit 65
    cat > "$out" <<JSON
{"schema_version":"resume-ir.model-manifest.v1","model_pack_id":"synthetic-pack","models":[{"id":"$model_id","type":"embedding","format":"safetensors","dim":$dimension,"artifact":{"path":"$artifact","sha256":"synthetic"},"license":{"id":"$license","reviewed":true}}]}
JSON
    printf 'model manifest draft: written\npaths: <redacted>\n'
    ;;
  validate-manifest)
    printf 'model manifest: valid\npaths: <redacted>\n'
    ;;
  *)
    printf 'unexpected fake resume-cli model command\n' >&2
    exit 64
    ;;
esac
SH
chmod 700 "$fake_resume_cli"

manifest_out="$tmpdir/PRIVATE-model-manifest.json"
manifest_stdout="$tmpdir/manifest-stdout.txt"
manifest_stderr="$tmpdir/manifest-stderr.txt"
FAKE_RESUME_CLI_ARGS="$fake_resume_cli_args" "$manifest_script" \
  --resume-cli "$fake_resume_cli" \
  --hf-cache-root "$hf_root" \
  --out "$manifest_out" \
  --model-id "sentence-transformers/all-MiniLM-L6-v2" \
  --model-pack-id "sentence-transformers-all-MiniLM-L6-v2-local" \
  --dimension 384 \
  --license Apache-2.0 \
  > "$manifest_stdout" 2> "$manifest_stderr"

if [ -s "$manifest_stderr" ]; then
  fail "local embedding manifest preparation wrote stderr on success"
fi
if [ ! -s "$manifest_out" ]; then
  fail "local embedding manifest preparation did not write manifest"
fi
require_text "$manifest_stdout" "embedding model manifest: written"
require_text "$manifest_stdout" "schema: resume-ir.model-manifest.v1"
require_text "$manifest_stdout" "model id: sentence-transformers/all-MiniLM-L6-v2"
require_text "$manifest_stdout" "dimension: 384"
require_text "$manifest_stdout" "license reviewed: yes"
require_text "$manifest_stdout" "license source: local model card"
require_text "$manifest_stdout" "paths: <redacted>"
require_text "$fake_resume_cli_args" "draft-manifest"
require_text "$fake_resume_cli_args" "--format safetensors"
require_text "$fake_resume_cli_args" "--reviewed"
require_text "$fake_resume_cli_args" "validate-manifest"
reject_text "$manifest_stdout" "$tmpdir" "temporary local path"
reject_text "$manifest_stderr" "$tmpdir" "temporary local path"
reject_text "$manifest_stdout" "PRIVATE-hf-cache" "private cache marker"
reject_text "$manifest_stderr" "PRIVATE-hf-cache" "private cache marker"
reject_text "$manifest_stdout" "SYNTHETIC MODEL WEIGHTS" "model bytes"

bad_hf_root="$tmpdir/PRIVATE-bad-hf-cache/hub"
bad_model_cache="$bad_hf_root/models--sentence-transformers--all-MiniLM-L6-v2"
bad_snapshot_dir="$bad_model_cache/snapshots/$snapshot"
mkdir -p "$bad_snapshot_dir" "$bad_model_cache/refs"
printf '%s\n' "$snapshot" > "$bad_model_cache/refs/main"
cat > "$bad_snapshot_dir/README.md" <<'EOF'
---
license: other-license
library_name: sentence-transformers
---
EOF
printf '%s\n' "SYNTHETIC MODEL WEIGHTS PLACEHOLDER" > "$bad_snapshot_dir/model.safetensors"
set +e
FAKE_RESUME_CLI_ARGS="$fake_resume_cli_args" "$manifest_script" \
  --resume-cli "$fake_resume_cli" \
  --hf-cache-root "$bad_hf_root" \
  --out "$tmpdir/PRIVATE-bad-model-manifest.json" \
  --model-id "sentence-transformers/all-MiniLM-L6-v2" \
  --model-pack-id "sentence-transformers-all-MiniLM-L6-v2-local" \
  --dimension 384 \
  --license Apache-2.0 \
  > "$tmpdir/bad-manifest-stdout.txt" 2> "$tmpdir/bad-manifest-stderr.txt"
bad_status=$?
set -e
if [ "$bad_status" -eq 0 ]; then
  fail "local embedding manifest preparation accepted mismatched model license"
fi
require_text "$tmpdir/bad-manifest-stderr.txt" "embedding model manifest blocked: local model license mismatch"
reject_text "$tmpdir/bad-manifest-stdout.txt" "$tmpdir" "temporary local path"
reject_text "$tmpdir/bad-manifest-stderr.txt" "$tmpdir" "temporary local path"
reject_text "$tmpdir/bad-manifest-stdout.txt" "PRIVATE-bad-hf-cache" "private cache marker"
reject_text "$tmpdir/bad-manifest-stderr.txt" "PRIVATE-bad-hf-cache" "private cache marker"

printf '%s\n' "local embedding runtime check passed"
