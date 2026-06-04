#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

if [ "$#" -ne 1 ]; then
  fail "usage: scripts/release/verify-macos-dmg.sh PATH_TO_DMG"
fi

dmg="$1"
[ -f "$dmg" ] || fail "missing dmg: $dmg"
command -v hdiutil >/dev/null 2>&1 || fail "hdiutil is required"

attempt=1
max_attempts=6
while [ "$attempt" -le "$max_attempts" ]; do
  if hdiutil verify "$dmg"; then
    exit 0
  fi
  if [ "$attempt" -eq "$max_attempts" ]; then
    fail "hdiutil verify failed after $max_attempts attempts"
  fi
  sleep_seconds=$((attempt * 2))
  printf 'hdiutil verify attempt %s failed; retrying in %ss\n' "$attempt" "$sleep_seconds" >&2
  sleep "$sleep_seconds"
  attempt=$((attempt + 1))
done
