#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GROK_SOURCE="${1:-}"
CUDA_ARCH="${2:-}"

if [[ -z "$GROK_SOURCE" || ! "$CUDA_ARCH" =~ ^[0-9]+$ ]]; then
    echo "usage: $0 /path/to/grok/source <cuda arch, for example 86>" >&2
    exit 2
fi

GROK_SOURCE="$(cd "$GROK_SOURCE" && pwd)"
if [[ ! -f "$GROK_SOURCE/extern/grok-gpu-plugin/CMakeLists.txt" ]]; then
    echo "$GROK_SOURCE has no extern/grok-gpu-plugin, run git submodule update --init extern/grok-gpu-plugin there" >&2
    exit 1
fi

IMAGE=postperfection-ubuntu24-build
UBUNTU_IMAGE=docker.io/library/ubuntu:24.04
CACHE_ROOT="${XDG_CACHE_HOME:-$HOME/.cache}/postperfection/ubuntu24"
GROK_BUILD="$CACHE_ROOT/grok-build-sm$CUDA_ARCH"
GROK_INSTALL="$CACHE_ROOT/grok-install-sm$CUDA_ARCH"
CARGO_CACHE="$CACHE_ROOT/cargo"
PNPM_STORE="$CACHE_ROOT/pnpm-store"
WORK="$CACHE_ROOT/work-imfwizard"
OUTPUT_DIRECTORY="$ROOT/gui/src-tauri/target/release/bundle/deb"

mkdir -p "$GROK_BUILD" "$GROK_INSTALL" "$CARGO_CACHE" "$PNPM_STORE" "$WORK" "$OUTPUT_DIRECTORY"

podman build --tag "$IMAGE" --file "$ROOT/scripts/ubuntu-deb.Containerfile" "$ROOT/scripts"

# :z would relabel every file in the grok and wizard trees for selinux
run_in_image() {
    podman run --rm -i --userns=keep-id --security-opt label=disable \
        -e HOME=/tmp -e CUDA_ARCH="$CUDA_ARCH" "$@"
}

run_in_image \
    -v "$GROK_SOURCE:/grok:ro" \
    -v "$GROK_BUILD:/grok-build" \
    -v "$GROK_INSTALL:/grok-install" \
    "$IMAGE" bash -euo pipefail <<'EOF'
if [[ ! -f /grok-build/CMakeCache.txt ]]; then
    cmake -S /grok -B /grok-build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=/grok-install \
        -DBUILD_SHARED_LIBS=ON \
        -DGRK_BUILD_PLUGIN_LOADER=ON \
        -DGRK_CUDA_ARCH="$CUDA_ARCH" \
        -DCMAKE_CUDA_ARCHITECTURES="$CUDA_ARCH" \
        -DGPUP_ENABLE_AUTH=ON \
        -DGPUP_USE_LEGACY_AUTH=ON \
        -DGPUP_ALLOW_DEV_LICENSE_KEY=ON \
        -DGRK_BUILD_CORE_SWIG_BINDINGS=OFF \
        -DGRK_BUILD_JPEG=OFF
fi
cmake --build /grok-build
cmake --install /grok-build
EOF

# --delete leaves the container's own target and node_modules in place
rsync -a --delete \
    --exclude .git \
    --exclude target/ \
    --exclude node_modules/ \
    --exclude /gui/dist/ \
    "$ROOT/" "$WORK/"

run_in_image \
    -v "$WORK:/wizard" \
    -v "$GROK_INSTALL:/grok-install:ro" \
    -v "$CARGO_CACHE:/cache/cargo" \
    -v "$PNPM_STORE:/cache/pnpm-store" \
    -e CARGO_HOME=/cache/cargo \
    "$IMAGE" bash -euo pipefail <<'EOF'
for candidate in /grok-install/lib64 /grok-install/lib; do
    if [[ -f "$candidate/libgrokj2k.so.1" && -f "$candidate/libgrokj2k_plugin.so" ]]; then
        GROK_LIBRARY_DIRECTORY="$candidate"
        break
    fi
done

if [[ -z "${GROK_LIBRARY_DIRECTORY:-}" ]]; then
    echo "no Grok core library and GPU plugin found under /grok-install" >&2
    exit 1
fi

# these win over the host grok paths in the committed cargo [env] config
export PKG_CONFIG_PATH="$GROK_LIBRARY_DIRECTORY/pkgconfig"
export LD_LIBRARY_PATH="$GROK_LIBRARY_DIRECTORY"

cd /wizard
cargo build --release -p imfwizard-cli --manifest-path rust/Cargo.toml
scripts/setup-tauri-bin.sh
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k.so.1" gui/src-tauri/libgrokj2k.so.1
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k_plugin.so" gui/src-tauri/libgrokj2k_plugin.so

CUBIN_ARCH="$(cuobjdump --list-elf gui/src-tauri/libgrokj2k_plugin.so | grep -oE 'sm_[0-9]+' | head -1 || true)"
if [[ "$CUBIN_ARCH" != "sm_$CUDA_ARCH" ]]; then
    echo "libgrokj2k_plugin.so carries ${CUBIN_ARCH:-no cubin}, expected sm_$CUDA_ARCH" >&2
    exit 1
fi

cd gui
pnpm install --frozen-lockfile --store-dir /cache/pnpm-store
# --config replaces arrays instead of appending
DEB_CONFIG="$(jq -c '{bundle: {linux: {deb: {
    depends: (.bundle.linux.deb.depends + ["libtiff6", "libcurl4t64"]),
    files: {
        "/usr/lib/imfwizard/libgrokj2k.so.1": "libgrokj2k.so.1",
        "/usr/lib/imfwizard/libgrokj2k_plugin.so": "libgrokj2k_plugin.so"
    }
}}}}' src-tauri/tauri.linux.conf.json)"
pnpm tauri build --bundles deb --config "$DEB_CONFIG"
EOF

read_tauri_config() {
    run_in_image -v "$WORK:/wizard:ro" "$IMAGE" jq -r "$1" /wizard/gui/src-tauri/tauri.conf.json
}
PRODUCT_NAME="$(read_tauri_config .productName)"
VERSION="$(read_tauri_config .version)"
BUILT_DEB="$WORK/gui/src-tauri/target/release/bundle/deb/${PRODUCT_NAME}_${VERSION}_amd64.deb"
DEB_NAME="${PRODUCT_NAME// /-}_${VERSION}-sm${CUDA_ARCH}_amd64.deb"
cp "$BUILT_DEB" "$OUTPUT_DIRECTORY/$DEB_NAME"
echo "wrote $OUTPUT_DIRECTORY/$DEB_NAME"

podman run --rm -i --security-opt label=disable \
    -v "$OUTPUT_DIRECTORY/$DEB_NAME:/debs/$DEB_NAME:ro" \
    -e DEB_NAME="$DEB_NAME" \
    -e DEBIAN_FRONTEND=noninteractive \
    "$UBUNTU_IMAGE" bash -euo pipefail <<'EOF'
apt-get update
apt-get install -y --no-install-recommends "/debs/$DEB_NAME"
command -v ffmpeg
imfwizard --version

if [[ "$(ldd /usr/bin/imfwizard-gui | grep -c "not found" || true)" != 0 ]]; then
    ldd /usr/bin/imfwizard-gui
    echo "imfwizard-gui has an unresolved library" >&2
    exit 1
fi

if ldd /usr/lib/imfwizard/libgrokj2k_plugin.so | grep "not found"; then
    echo "libgrokj2k_plugin.so has an unresolved library" >&2
    exit 1
fi

GRK_PLUGIN_PATH=/usr/lib/imfwizard imfwizard --version
EOF
echo "deb installs on ubuntu:24.04"
