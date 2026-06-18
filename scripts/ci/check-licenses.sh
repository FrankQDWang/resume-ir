#!/usr/bin/env sh
set -eu

CARGO_BIN="${CARGO:-}"
if [ -z "$CARGO_BIN" ]; then
  CARGO_BIN=cargo
fi
if ! "$CARGO_BIN" --version >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi
if ! "$CARGO_BIN" --version >/dev/null 2>&1; then
  printf '%s\n' "license check requires cargo" >&2
  exit 1
fi

metadata_file="$(mktemp)"
trap 'rm -f "$metadata_file"' EXIT

"$CARGO_BIN" metadata --format-version 1 --locked > "$metadata_file"

python3 - "$metadata_file" <<'PY'
import json
import re
import sys

allowed = {
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CC0-1.0",
    "ISC",
    "LLVM-exception",
    "MIT",
    "MPL-2.0",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "Unlicense",
    "Zlib",
    "zlib-acknowledgement",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "LGPL-3.0-only",
    "LGPL-3.0-or-later",
}
forbidden_tokens = {"AGPL", "AGPL-3.0-only", "AGPL-3.0-or-later", "SSPL", "SSPL-1.0"}


def tokens_for(expression):
    return {
        token
        for token in re.split(r"\s+|\(|\)|/|\+|,|AND|WITH", expression)
        if token and token != "OR"
    }


def is_gpl_compatible_choice(expression):
    for choice in re.split(r"\s+OR\s+", expression):
        tokens = tokens_for(choice)
        if not tokens:
            continue
        if any(token in forbidden_tokens for token in tokens):
            continue
        if all(token in allowed for token in tokens):
            return True
    return False

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    metadata = json.load(handle)

failures = []
for package in metadata.get("packages", []):
    name = package["name"]
    license_expr = (package.get("license") or "").strip()
    license_file = package.get("license_file")
    source = package.get("source")

    if not license_expr:
        if source is None and license_file:
            continue
        failures.append(f"{name}: missing license expression")
        continue

    if not is_gpl_compatible_choice(license_expr):
        tokens = sorted(tokens_for(license_expr))
        failures.append(
            f"{name}: no reviewed GPL-compatible license choice in {license_expr!r}; tokens={', '.join(tokens)}"
        )

if failures:
    for failure in failures:
        print(failure, file=sys.stderr)
    sys.exit(1)

print("license check passed")
PY
