#!/bin/bash
set -euo pipefail

readonly TESSERACT_COMMIT=6e1d56a847e697de07b38619356550e5cf4e8633
readonly LEPTONICA_COMMIT=13275a278eb55b5746e33f95fbf5a2c8f604b3ab
readonly TESSDATA_COMMIT=65727574dfcd264acbb0c3e07860e4e9e9b22185
readonly TESSERACT_LICENSE_SHA=cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30
readonly LEPTONICA_LICENSE_SHA=87829abb5bbb00b55a107365da89e9a33f86c4250169e5a1e5588505be7d5806
readonly ENG_SHA=7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2
readonly CHI_SIM_SHA=a5fcb6f0db1e1d6d8522f39db4e848f05984669172e584e8d76b6b3141e1f730
readonly TSV_SHA=59d079bb75d8b3d7c839a3564580cb559e362c93a9d70f234e421c0c3e767e04

mkdir -p /src /build /opt/windows /out/runtime /out/data/configs
git clone --quiet --filter=blob:none https://github.com/DanBloomberg/leptonica /src/leptonica
git -C /src/leptonica checkout --quiet "$LEPTONICA_COMMIT"
git clone --quiet --filter=blob:none https://github.com/tesseract-ocr/tesseract /src/tesseract
git -C /src/tesseract checkout --quiet "$TESSERACT_COMMIT"
for source in /src/leptonica /src/tesseract; do
  test -z "$(git -C "$source" status --porcelain --untracked-files=all)"
done

cmake -S /src/leptonica -B /build/leptonica -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE=/builder/toolchain.cmake \
  -DCMAKE_POLICY_DEFAULT_CMP0091=NEW \
  -DCMAKE_INSTALL_PREFIX=/opt/windows \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded \
  -DSW_BUILD=OFF \
  -DENABLE_PNG=OFF \
  -DBUILD_PROG=OFF \
  -DENABLE_ZLIB=OFF \
  -DENABLE_GIF=OFF \
  -DENABLE_JPEG=OFF \
  -DENABLE_TIFF=OFF \
  -DENABLE_WEBP=OFF \
  -DENABLE_OPENJPEG=OFF
grep -q -- '-MT' /build/leptonica/build.ninja
! grep -q -- '/MD' /build/leptonica/build.ninja
cmake --build /build/leptonica --parallel 8
cmake --install /build/leptonica

cmake -S /src/tesseract -B /build/tesseract -G Ninja \
  -DCMAKE_TOOLCHAIN_FILE=/builder/toolchain.cmake \
  -DCMAKE_POLICY_DEFAULT_CMP0091=NEW \
  -DLeptonica_DIR=/opt/windows/lib/cmake/leptonica \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded \
  -DSW_BUILD=OFF \
  -DBUILD_TRAINING_TOOLS=OFF \
  -DGRAPHICS_DISABLED=ON \
  -DOPENMP_BUILD=OFF \
  -DDISABLE_TIFF=ON \
  -DLEPT_TIFF_RESULT=1 \
  -DLEPT_TIFF_COMPILE_SUCCESS=TRUE \
  -DDISABLE_ARCHIVE=ON \
  -DDISABLE_CURL=ON \
  -DENABLE_LTO=ON \
  -DFAST_FLOAT=ON
grep -q -- '-MT' /build/tesseract/build.ninja
! grep -q -- '/MD' /build/tesseract/build.ninja
grep -q -- '-flto' /build/tesseract/build.ninja
cmake --build /build/tesseract --target tesseract --parallel 8

mapfile -t tesseract_bins < <(
  find /build/tesseract -type f -iname tesseract.exe -print
)
test "${#tesseract_bins[@]}" -eq 1
mapfile -t artifact_imports < <(
  llvm-readobj-19 --coff-imports "${tesseract_bins[0]}" \
    | sed -n 's/^[[:space:]]*Name: //p' \
    | tr '[:lower:]' '[:upper:]' \
    | sort -u
)
test "${#artifact_imports[@]}" -eq 1
test "${artifact_imports[0]}" = KERNEL32.DLL
cp "${tesseract_bins[0]}" /out/runtime/tesseract.exe
cp /src/tesseract/LICENSE /out/runtime/LICENSE
cp /src/leptonica/leptonica-license.txt /out/runtime/leptonica-license.txt
cp /src/tesseract/tessdata/configs/tsv /out/data/configs/tsv
curl -fsSL "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/$TESSDATA_COMMIT/eng.traineddata" -o /out/data/eng.traineddata
curl -fsSL "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/$TESSDATA_COMMIT/chi_sim.traineddata" -o /out/data/chi_sim.traineddata
curl -fsSL "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/$TESSDATA_COMMIT/LICENSE" -o /out/data/LICENSE

printf '%s  %s\n' \
  "$TESSERACT_LICENSE_SHA" /out/runtime/LICENSE \
  "$LEPTONICA_LICENSE_SHA" /out/runtime/leptonica-license.txt \
  "$ENG_SHA" /out/data/eng.traineddata \
  "$CHI_SIM_SHA" /out/data/chi_sim.traineddata \
  "$TSV_SHA" /out/data/configs/tsv \
  "$TESSERACT_LICENSE_SHA" /out/data/LICENSE \
  | sha256sum -c -

printf '%s\n' \
  '{' \
  '  "schema_version": "resume-ir.windows-ocr-compile-receipt.v1",' \
  '  "target_triple": "x86_64-pc-windows-msvc",' \
  '  "msvc_runtime": "static",' \
  '  "lto": true,' \
  '  "source_trees_clean": true,' \
  '  "artifact_imports": ["KERNEL32.DLL"],' \
  '  "compile_passed": true,' \
  '  "native_smoke_passed": false' \
  '}' > /out/compile-receipt.json
test "$(find /out -type f | wc -l)" -eq 8
test -z "$(find /out -type l -print -quit)"
