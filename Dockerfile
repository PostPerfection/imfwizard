# IMF Wizard — Headless batch processing container
# Usage:
#   docker build -t imfwizard .
#   docker run -v /path/to/media:/data imfwizard create --video /data/j2k --audio /data/audio.wav --output /data/out
#
# For REST API mode:
#   docker run -p 8081:8081 -v /path/to/media:/data imfwizard serve --bind 0.0.0.0:8081

FROM rust:1-bookworm AS builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    libxml2-dev \
    libxerces-c-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

RUN cargo build --release --manifest-path rust/Cargo.toml -p imfwizard-cli

# --- Runtime image ---
FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 \
    ffmpeg \
    openjpeg-tools \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/rust/target/release/imfwizard /usr/local/bin/imfwizard

# Create non-root user for security
RUN useradd -m -s /bin/bash imfwizard
USER imfwizard
WORKDIR /data

EXPOSE 8080

ENTRYPOINT ["imfwizard"]
CMD ["--help"]
