# Headless imfwizard. Build: docker build -t imfwizard .
# Create:   docker run -v /path/to/media:/data imfwizard create --title "My Film" --video /data/master.mov --audio /data/audio.wav --output /data/out
# REST API: docker run -p 8081:8081 -v /path/to/media:/data imfwizard serve --bind 0.0.0.0:8081 --api-key <key>
# Watch:    docker run -v /path/to/incoming:/in -v /path/to/out:/out imfwizard watch /in --output /out
# Photon validation needs a JRE and the photon jars mounted, set PHOTON_JAR to their path.

ARG GROK_REF=v20.4.14
ARG FFMPEG_MPV_RELEASE=v1.0.0
ARG FFMPEG_URL=https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-linux64-gpl-8.1.tar.xz

FROM ubuntu:24.04 AS grok
ARG GROK_REF
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN git init -q /tmp/grok-src \
    && git -C /tmp/grok-src fetch --depth 1 https://github.com/GrokImageCompression/grok.git "$GROK_REF" \
    && git -C /tmp/grok-src checkout -q FETCH_HEAD \
    && git -C /tmp/grok-src submodule update --init --depth 1 \
    && cmake -S /tmp/grok-src -B /tmp/grok-build \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/grok \
        -DGRK_BUILD_CORE_SWIG_BINDINGS=OFF \
    && cmake --build /tmp/grok-build --parallel "$(nproc)" \
    && cmake --install /tmp/grok-build \
    && mkdir -p /opt/grok-runtime \
    && cp -a /opt/grok/lib*/libgrokj2k*.so* /opt/grok-runtime/

FROM ubuntu:24.04 AS builder
ARG FFMPEG_MPV_RELEASE
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake curl ca-certificates pkg-config git libclang-dev xz-utils \
    libssl-dev libxml2-dev libxerces-c-dev libasound2-dev \
    && rm -rf /var/lib/apt/lists/*
RUN archive=ffmpeg-mpv-linux-x86_64.tar.xz \
    && base="https://github.com/PostPerfection/ffmpeg-mpv-builds/releases/download/$FFMPEG_MPV_RELEASE" \
    && mkdir -p /tmp/ffmpeg-mpv-download /opt/ffmpeg-mpv \
    && cd /tmp/ffmpeg-mpv-download \
    && curl -fsSL --retry 5 --retry-all-errors -o SHA256SUMS "$base/SHA256SUMS" \
    && curl -fsSL --retry 5 --retry-all-errors -o "$archive" "$base/$archive" \
    && grep "  $archive\$" SHA256SUMS > expected.sha256 \
    && sha256sum -c expected.sha256 \
    && tar -C /opt/ffmpeg-mpv --strip-components=1 -xJf "$archive" \
    && rm -rf /tmp/ffmpeg-mpv-download
RUN apt-get update \
    && xargs apt-get install -y --no-install-recommends < /opt/ffmpeg-mpv/ubuntu-24.04-runtime-packages.txt \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH
COPY --from=grok /opt/grok /opt/grok
ENV PKG_CONFIG_PATH=/opt/ffmpeg-mpv/lib/pkgconfig:/opt/grok/lib/pkgconfig:/opt/grok/lib64/pkgconfig
WORKDIR /src
COPY . .
RUN cargo build --release -p imfwizard-cli --manifest-path rust/Cargo.toml

FROM ubuntu:24.04
ARG FFMPEG_URL
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3t64 libxml2 libxerces-c3.2t64 libasound2t64 xmlsec1 fonts-dejavu-core ca-certificates curl xz-utils \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /opt/ffmpeg-mpv/ubuntu-24.04-runtime-packages.txt /tmp/ffmpeg-mpv-runtime-packages.txt
RUN apt-get update \
    && xargs apt-get install -y --no-install-recommends < /tmp/ffmpeg-mpv-runtime-packages.txt \
    && rm -rf /var/lib/apt/lists/* /tmp/ffmpeg-mpv-runtime-packages.txt
RUN curl -fsSL --retry 5 --retry-all-errors -o /tmp/ffmpeg.tar.xz "$FFMPEG_URL" \
    && tar -xJf /tmp/ffmpeg.tar.xz -C /tmp \
    && install -m 755 /tmp/ffmpeg-*/bin/ffmpeg /tmp/ffmpeg-*/bin/ffprobe /usr/local/bin/ \
    && rm -rf /tmp/ffmpeg.tar.xz /tmp/ffmpeg-*
COPY --from=grok /opt/grok-runtime/ /usr/local/lib/
COPY --from=builder /opt/ffmpeg-mpv/lib/ /opt/ffmpeg-mpv/lib/
RUN echo /opt/ffmpeg-mpv/lib > /etc/ld.so.conf.d/ffmpeg-mpv.conf && ldconfig
COPY --from=builder /src/rust/target/release/imfwizard /usr/local/bin/imfwizard
RUN useradd -m -s /bin/bash imfwizard
USER imfwizard
WORKDIR /data
EXPOSE 8081
ENTRYPOINT ["imfwizard"]
CMD ["--help"]
