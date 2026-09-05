#!/usr/bin/env bash
# Build the imfwizard CLI and copy it into the Tauri binaries folder
# so that `npx tauri dev` / `npx tauri build` can find it.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST_DIR="${ROOT}/rust"
BIN_DIR="${ROOT}/gui/src-tauri"
TARGET_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"

# Build the Rust CLI if needed
if [[ ! -f "${RUST_DIR}/target/release/imfwizard" ]]; then
    echo "Building imfwizard CLI …"
    cargo build --release -p imfwizard-cli --manifest-path "${RUST_DIR}/Cargo.toml"
fi

mkdir -p "${BIN_DIR}"
cp "${RUST_DIR}/target/release/imfwizard" "${BIN_DIR}/imfwizard-${TARGET_TRIPLE}"
echo "Installed: ${BIN_DIR}/imfwizard-${TARGET_TRIPLE}"
