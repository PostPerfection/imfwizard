#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GROK_INSTALL="${1:-}"
FFMPEG_MPV_DIR="${2:-${FFMPEG_MPV_DIR:-}}"

if [[ -z "$GROK_INSTALL" || -z "$FFMPEG_MPV_DIR" ]]
then
    echo "usage: $0 /path/to/grok/install [/path/to/ffmpeg-mpv-linux-x86_64, default \$FFMPEG_MPV_DIR]" >&2
    exit 2
fi

for candidate in "$GROK_INSTALL/lib64" "$GROK_INSTALL/lib"; do
    if [[ -f "$candidate/libgrokj2k.so.1" && -f "$candidate/libgrokj2k_plugin.so" ]]; then
        GROK_LIBRARY_DIRECTORY="$candidate"
        break
    fi
done

if [[ -z "${GROK_LIBRARY_DIRECTORY:-}" ]]; then
    echo "no Grok core library and GPU plugin found under $GROK_INSTALL" >&2
    exit 1
fi

export PKG_CONFIG_PATH="$FFMPEG_MPV_DIR/lib/pkgconfig:$GROK_LIBRARY_DIRECTORY/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LD_LIBRARY_PATH="$FFMPEG_MPV_DIR/lib:$GROK_LIBRARY_DIRECTORY${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

cargo build --release -p imfwizard-cli --manifest-path "$ROOT/rust/Cargo.toml"
"$ROOT/scripts/setup-tauri-bin.sh"
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k.so.1" "$ROOT/gui/src-tauri/libgrokj2k.so.1"
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k_plugin.so" "$ROOT/gui/src-tauri/libgrokj2k_plugin.so"

CUDA_ARCH="$(cuobjdump --list-elf "$ROOT/gui/src-tauri/libgrokj2k_plugin.so" | grep -oE 'sm_[0-9]+' | head -1)"
if [[ -z "$CUDA_ARCH" ]]; then
    echo "cuobjdump found no cubin in libgrokj2k_plugin.so" >&2
    exit 1
fi
RPM_RELEASE="1.${CUDA_ARCH/_/}"

FFMPEG_MPV_FILES="$("$ROOT/scripts/ffmpeg-mpv-bundle-config.sh" "$FFMPEG_MPV_DIR" Linux)"

cd "$ROOT/gui"
pnpm install --frozen-lockfile
# the plugin is private, the committed config leaves it out
PLUGIN_FILES='{"bundle":{"linux":{"rpm":{"release":"'"$RPM_RELEASE"'","files":{"/usr/lib/imfwizard/libgrokj2k.so.1":"libgrokj2k.so.1","/usr/lib/imfwizard/libgrokj2k_plugin.so":"libgrokj2k_plugin.so"}}}}}'
pnpm tauri build --bundles rpm --config "$FFMPEG_MPV_FILES" --config "$PLUGIN_FILES"

# tauri names the file after the product name, which has a space
shopt -s nullglob
for rpm in "$ROOT"/gui/src-tauri/target/release/bundle/rpm/*\ *-"$RPM_RELEASE".*.rpm; do
    mv "$rpm" "${rpm// /-}"
    echo "wrote ${rpm// /-}"
done
