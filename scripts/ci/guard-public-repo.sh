#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  fail "public repo guard must run inside a git work tree"
fi

tracked_sensitive_paths=$(
  git ls-files \
    | grep -E '(^|/)(local-data|data|resume-data|resumes|corpus|corpora|indexes|logs|diagnostics|bench-results|model-cache|\.cache)(/|$)|\.(sqlite|sqlite3|db|log|profraw)$|(^|/)(\.env(\..*)?|ipc\.auth|[^/]*\.(token|pem|key))$' \
    | grep -v '^\.env\.example$' \
    | grep -v '^tests/fixtures/resumes/' || true
)

if [ -n "$tracked_sensitive_paths" ]; then
  printf '%s\n' "$tracked_sensitive_paths" >&2
  fail "tracked sensitive local artifacts are not allowed in the public repository"
fi

tracked_model_artifacts=$(
  git ls-files | grep -E '(^|/)models/.+\.(bin|onnx|safetensors|pt|gguf)$' || true
)

if [ -n "$tracked_model_artifacts" ]; then
  printf '%s\n' "$tracked_model_artifacts" >&2
  fail "tracked model weight/cache artifacts are not allowed without a reviewed manifest"
fi

token_pattern='ghp_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]+|sk-[A-Za-z0-9_-]{20,}|hf_[A-Za-z0-9]{20,}|BEGIN [A-Z ]*PRIVATE KEY'
token_matches=$(
  rg -n --pcre2 --hidden \
    --glob '!.git/**' \
    --glob '!target/**' \
    --glob '!local-data/**' \
    --glob '!data/**' \
    --glob '!resume-data/**' \
    --glob '!resumes/**' \
    --glob '!diagnostics/**' \
    --glob '!logs/**' \
    --glob '!model-cache/**' \
    "$token_pattern" . || true
)

if [ -n "$token_matches" ]; then
  printf '%s\n' "$token_matches" >&2
  fail "possible credential or private key material found"
fi

printf '%s\n' "public repo guard passed"
