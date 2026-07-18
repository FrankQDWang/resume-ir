#!/bin/bash
set -euo pipefail

test "$(uname -m)" = x86_64
if grep -q 'VirtualApple' /proc/cpuinfo; then
  echo 'Windows OCR native smoke requires a native x86_64 Linux builder' >&2
  exit 86
fi

export WINEDEBUG=-all
export WINEPREFIX=/wine
timeout 30s wine wineboot --init >/dev/null 2>&1
convert -size 1000x220 xc:white -font DejaVu-Sans -pointsize 84 \
  -fill black -draw "text 35,145 'RESUME IR 2468'" /out/smoke.ppm
test "$(head -c 2 /out/smoke.ppm)" = P6
test "$(wc -c < /out/smoke.ppm)" -le 33554432
TESSDATA_PREFIX='Z:\out\data' timeout 30s wine /out/runtime/tesseract.exe \
  'Z:\out\smoke.ppm' stdout --psm 6 -l eng+chi_sim tsv \
  > /out/smoke.tsv 2>/dev/null
test "$(wc -c < /out/smoke.tsv)" -le 4194304
grep -q $'\tRESUME$' /out/smoke.tsv
grep -q $'\t2468$' /out/smoke.tsv
rm /out/smoke.ppm /out/smoke.tsv

printf '%s\n' \
  '{' \
  '  "schema_version": "resume-ir.windows-ocr-native-smoke.v1",' \
  '  "host": "linux/amd64-native",' \
  '  "input_format": "ppm-p6-rgb8",' \
  '  "languages": ["eng", "chi_sim"],' \
  '  "output_format": "tesseract-tsv",' \
  '  "passed": true' \
  '}' > /out/native-smoke.json
