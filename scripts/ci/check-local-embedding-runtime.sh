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
hash_runtime="scripts/local/embedding-runtime-hash.py"
e5_runtime="scripts/local/embedding-runtime-e5-onnx.py"
manifest_script="scripts/local/prepare-local-embedding-model-manifest.sh"
if [ ! -f "$runtime" ]; then
  fail "missing local embedding runtime adapter"
fi
if [ ! -f "$hash_runtime" ]; then
  fail "missing lightweight local embedding runtime adapter"
fi
if [ ! -f "$e5_runtime" ]; then
  fail "missing local multilingual E5 ONNX embedding runtime adapter"
fi
if [ ! -f "$manifest_script" ]; then
  fail "missing local embedding model manifest preparation script"
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-local-embedding-runtime.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

hash_input_file="$tmpdir/PRIVATE-hash-embedding-input.txt"
cat >"$hash_input_file" <<'EOF'
resume-ir-embedding-input-v1
model_id=resume-ir-hash-embedding-v1
dimension=8
count=2
input=doc-private-hash-1	32
text:
PRIVATE hash raw resume text rust ml
--resume-ir-embedding-input-boundary--
input=doc-private-hash-2	37
text:
PRIVATE hash raw resume text python ml
--resume-ir-embedding-input-boundary--
EOF

hash_stdout_file="$tmpdir/hash-stdout.txt"
hash_stdout_repeat_file="$tmpdir/hash-stdout-repeat.txt"
hash_stderr_file="$tmpdir/hash-stderr.txt"
RESUME_IR_EMBEDDING_INPUT_PATH="$hash_input_file" \
RESUME_IR_EMBEDDING_MODEL_ID="resume-ir-hash-embedding-v1" \
RESUME_IR_EMBEDDING_DIMENSION=8 \
python3 "$hash_runtime" >"$hash_stdout_file" 2>"$hash_stderr_file"
RESUME_IR_EMBEDDING_INPUT_PATH="$hash_input_file" \
RESUME_IR_EMBEDDING_MODEL_ID="resume-ir-hash-embedding-v1" \
RESUME_IR_EMBEDDING_DIMENSION=8 \
python3 "$hash_runtime" >"$hash_stdout_repeat_file" 2>"$tmpdir/hash-stderr-repeat.txt"

if [ -s "$hash_stderr_file" ]; then
  fail "lightweight local embedding runtime wrote stderr on success"
fi
cmp "$hash_stdout_file" "$hash_stdout_repeat_file" >/dev/null 2>&1 \
  || fail "lightweight local embedding runtime is not deterministic"
require_text "$hash_stdout_file" "resume-ir-embedding-v1"
require_text "$hash_stdout_file" "model_id=resume-ir-hash-embedding-v1"
require_text "$hash_stdout_file" "dimension=8"
require_text "$hash_stdout_file" "vector=doc-private-hash-1	"
require_text "$hash_stdout_file" "vector=doc-private-hash-2	"
awk -F '\t' '
  /^vector=/ {
    split($2, values, ",")
    if (length(values) != 8) {
      exit 1
    }
    nonzero = 0
    for (i = 1; i <= 8; i++) {
      if (values[i] != "0") {
        nonzero = 1
      }
    }
    if (nonzero == 0) {
      exit 1
    }
    vectors += 1
  }
  END {
    if (vectors != 2) {
      exit 1
    }
  }
' "$hash_stdout_file" \
  || fail "lightweight local embedding runtime returned invalid vectors"
reject_text "$hash_stdout_file" "PRIVATE hash raw resume text" "raw hash embedding input text"
reject_text "$hash_stdout_file" "$tmpdir" "temporary local path"
reject_text "$hash_stdout_file" "$hash_input_file" "input path"

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

mkdir -p "$tmpdir/transformers" "$tmpdir/PRIVATE-e5-model/onnx"
printf '%s\n' "SYNTHETIC ONNX MODEL PLACEHOLDER" > "$tmpdir/PRIVATE-e5-model/onnx/model.onnx"
cat >"$tmpdir/transformers/__init__.py" <<'PY'
import os
import numpy as np

class _Tokenizer:
    def __call__(self, texts, max_length=512, padding=True, truncation=True, return_tensors=None):
        if max_length != 512 or not padding or not truncation or return_tensors != "np":
            raise RuntimeError("adapter used unexpected tokenizer arguments")
        labels = []
        input_ids = np.zeros((len(texts), 3), dtype=np.int64)
        attention_mask = np.ones((len(texts), 3), dtype=np.int64)
        for index, text in enumerate(texts):
            if text.startswith("query: "):
                labels.append("query")
                input_ids[index, 0] = 1
            elif text.startswith("passage: "):
                labels.append("passage")
                input_ids[index, 0] = 2
            else:
                raise RuntimeError("missing E5 retrieval prefix")
        prefix_log = os.environ.get("RESUME_IR_E5_TEST_PREFIX_LOG")
        if prefix_log:
            with open(prefix_log, "w", encoding="utf-8") as handle:
                handle.write("\n".join(labels))
                handle.write("\n")
        return {"input_ids": input_ids, "attention_mask": attention_mask}

class AutoTokenizer:
    @staticmethod
    def from_pretrained(model_dir, local_files_only=True, use_fast=True):
        if not local_files_only:
            raise RuntimeError("adapter must load tokenizer local-only")
        if not use_fast:
            raise RuntimeError("adapter must request the fast local tokenizer")
        return _Tokenizer()
PY
cat >"$tmpdir/onnxruntime.py" <<'PY'
import numpy as np

class SessionOptions:
    pass

class _Input:
    def __init__(self, name):
        self.name = name

class InferenceSession:
    def __init__(self, model_path, sess_options=None, providers=None):
        if providers != ["CPUExecutionProvider"]:
            raise RuntimeError("adapter must default to CPUExecutionProvider")
        self.model_path = model_path

    def get_inputs(self):
        return [_Input("input_ids"), _Input("attention_mask")]

    def run(self, output_names, feed):
        input_ids = feed["input_ids"]
        batch, seq_len = input_ids.shape
        hidden = np.zeros((batch, seq_len, 384), dtype=np.float32)
        for row in range(batch):
            hidden[row, :, 0] = float(input_ids[row, 0])
            hidden[row, :, 1] = float(row + 1)
        return [hidden]
PY

e5_input_file="$tmpdir/PRIVATE-e5-embedding-input.txt"
cat >"$e5_input_file" <<'EOF'
resume-ir-embedding-input-v1
model_id=intfloat/multilingual-e5-small
dimension=384
count=2
input=query	26
text:
PRIVATE e5 keyword query
--resume-ir-embedding-input-boundary--
input=doc-private-e5-1	25
text:
PRIVATE e5 resume passage
--resume-ir-embedding-input-boundary--
EOF

e5_stdout_file="$tmpdir/e5-stdout.txt"
e5_stdout_repeat_file="$tmpdir/e5-stdout-repeat.txt"
e5_stderr_file="$tmpdir/e5-stderr.txt"
e5_prefix_log="$tmpdir/e5-prefix-log.txt"
PYTHONPATH="$tmpdir" \
RESUME_IR_EMBEDDING_INPUT_PATH="$e5_input_file" \
RESUME_IR_EMBEDDING_MODEL_ID="intfloat/multilingual-e5-small" \
RESUME_IR_EMBEDDING_DIMENSION=384 \
RESUME_IR_E5_MODEL_DIR="$tmpdir/PRIVATE-e5-model" \
RESUME_IR_E5_TEST_PREFIX_LOG="$e5_prefix_log" \
python3 "$e5_runtime" >"$e5_stdout_file" 2>"$e5_stderr_file"
PYTHONPATH="$tmpdir" \
RESUME_IR_EMBEDDING_INPUT_PATH="$e5_input_file" \
RESUME_IR_EMBEDDING_MODEL_ID="intfloat/multilingual-e5-small" \
RESUME_IR_EMBEDDING_DIMENSION=384 \
RESUME_IR_E5_MODEL_DIR="$tmpdir/PRIVATE-e5-model" \
python3 "$e5_runtime" >"$e5_stdout_repeat_file" 2>"$tmpdir/e5-stderr-repeat.txt"

if [ -s "$e5_stderr_file" ]; then
  fail "local multilingual E5 ONNX runtime wrote stderr on success"
fi
cmp "$e5_stdout_file" "$e5_stdout_repeat_file" >/dev/null 2>&1 \
  || fail "local multilingual E5 ONNX runtime is not deterministic"
require_text "$e5_stdout_file" "resume-ir-embedding-v1"
require_text "$e5_stdout_file" "model_id=intfloat/multilingual-e5-small"
require_text "$e5_stdout_file" "dimension=384"
require_text "$e5_stdout_file" "vector=query	"
require_text "$e5_stdout_file" "vector=doc-private-e5-1	"
require_text "$e5_prefix_log" "query"
require_text "$e5_prefix_log" "passage"
awk -F '\t' '
  /^vector=/ {
    split($2, values, ",")
    if (length(values) != 384) {
      exit 1
    }
    nonzero = 0
    for (i = 1; i <= 384; i++) {
      if (values[i] != "0") {
        nonzero = 1
      }
    }
    if (nonzero == 0) {
      exit 1
    }
    vectors += 1
  }
  END {
    if (vectors != 2) {
      exit 1
    }
  }
' "$e5_stdout_file" \
  || fail "local multilingual E5 ONNX runtime returned invalid vectors"
reject_text "$e5_stdout_file" "PRIVATE e5" "raw E5 embedding input text"
reject_text "$e5_stdout_file" "$tmpdir" "temporary local path"
reject_text "$e5_stdout_file" "$e5_input_file" "input path"
reject_text "$e5_prefix_log" "PRIVATE e5" "raw E5 embedding input text"

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
