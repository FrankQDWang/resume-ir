FROM rust:1.81.0-slim-bookworm@sha256:f9fb6bdb0483de4ade93b262a3f6cf8c2985fca1d34784914bbcabd5a34d3197 AS toolchain

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]
ENV DEBIAN_FRONTEND=noninteractive
ARG TARGETARCH

RUN test "$TARGETARCH" = amd64 \
  && dpkg --add-architecture i386 \
  && apt-get update \
  && apt-get install -y --no-install-recommends \
    ca-certificates curl git gpg tar cmake ninja-build imagemagick fonts-dejavu-core \
  && mkdir -p /usr/local/share/keyrings \
  && curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key \
    | gpg --dearmor >/usr/local/share/keyrings/llvm.gpg \
  && curl -fsSL https://dl.winehq.org/wine-builds/winehq.key \
    | gpg --dearmor >/usr/local/share/keyrings/winehq.gpg \
  && echo 'deb [signed-by=/usr/local/share/keyrings/llvm.gpg] http://apt.llvm.org/bookworm/ llvm-toolchain-bookworm-19 main' \
    >/etc/apt/sources.list.d/llvm.list \
  && echo 'deb [signed-by=/usr/local/share/keyrings/winehq.gpg] https://dl.winehq.org/wine-builds/debian/ bookworm main' \
    >/etc/apt/sources.list.d/winehq.list \
  && apt-get update \
  && apt-get install -y --no-install-recommends clang-19 llvm-19 lld-19 winehq-staging \
  && ln -s clang-19 /usr/bin/clang-cl \
  && ln -s llvm-lib-19 /usr/bin/llvm-lib \
  && ln -s lld-link-19 /usr/bin/lld-link \
  && ln -s llvm-rc-19 /usr/bin/llvm-rc \
  && ln -s llvm-mt-19 /usr/bin/llvm-mt \
  && test "$(clang-cl --version | sed -n '1s/.*version \([0-9.]*\).*/\1/p')" = 19.1.7 \
  && test "$(cmake --version | sed -n '1s/.* //p')" = 3.25.1 \
  && test "$(ninja --version)" = 1.11.1 \
  && test "$(wine --version | sed -n 's/^wine-\([0-9.]*\).*/\1/p')" = 11.10 \
  && rm -rf /var/lib/apt/lists/*

RUN --mount=type=cache,target=/xwin-cache \
  curl -fsSL https://github.com/Jake-Shadle/xwin/releases/download/0.9.0/xwin-0.9.0-x86_64-unknown-linux-musl.tar.gz -o /tmp/xwin.tar.gz \
  && echo '31e1033f30608ba6b821d17f1461042bd54c23424813c9b4e9ae15b6d32fa4cd  /tmp/xwin.tar.gz' | sha256sum -c - \
  && tar -xzf /tmp/xwin.tar.gz -C /usr/local/bin --strip-components=1 \
    xwin-0.9.0-x86_64-unknown-linux-musl/xwin \
  && rm -rf /xwin-cache/output /xwin \
  && xwin --accept-license --arch x86_64 --sdk-version 10.0.26100 \
    --crt-version 14.44.17.14 --http-retry 3 --timeout 120s \
    --cache-dir /xwin-cache splat --output /xwin-cache/output \
  && cp -a /xwin-cache/output /xwin \
  && test -f /xwin/sdk/lib/um/x86_64/WS2_32.Lib \
  && test ! -e /xwin/sdk/lib/um/x86_64/Ws2_32.lib \
  && ln -s WS2_32.Lib /xwin/sdk/lib/um/x86_64/Ws2_32.lib \
  && rm /tmp/xwin.tar.gz /usr/local/bin/xwin

FROM toolchain AS compile
COPY toolchain.cmake /builder/toolchain.cmake
COPY build.sh /builder/build.sh
RUN chmod 0755 /builder/build.sh && /builder/build.sh

FROM compile AS native-smoke
COPY smoke.sh /builder/smoke.sh
RUN chmod 0755 /builder/smoke.sh && /builder/smoke.sh

FROM scratch AS compile-output
COPY --from=compile /out /

FROM scratch AS validated-output
COPY --from=native-smoke /out /
