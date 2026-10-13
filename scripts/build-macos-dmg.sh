#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GROK_INSTALL="${1:-}"
FFMPEG_MPV_DIR="${2:-${FFMPEG_MPV_DIR:-}}"

if [[ -z "$GROK_INSTALL" || -z "$FFMPEG_MPV_DIR" ]]
then
    echo "usage: $0 /path/to/grok/install [/path/to/ffmpeg-mpv-macos-arm64, default \$FFMPEG_MPV_DIR]" >&2
    exit 2
fi

for candidate in "$GROK_INSTALL/lib" "$GROK_INSTALL/lib64"; do
    if [[ -f "$candidate/libgrokj2k.1.dylib" && -f "$candidate/libgrokj2k_plugin.dylib" && -f "$candidate/grok_kernels.metallib" ]]; then
        GROK_LIBRARY_DIRECTORY="$candidate"
        break
    fi
done

if [[ -z "${GROK_LIBRARY_DIRECTORY:-}" ]]; then
    echo "no Grok core library, GPU plugin and Metal kernels found under $GROK_INSTALL" >&2
    exit 1
fi

case "$(uname -m)" in
    arm64) DMG_ARCH="aarch64" ;;
    x86_64) DMG_ARCH="x64" ;;
    *)
        echo "tauri names no dmg for $(uname -m)" >&2
        exit 1
        ;;
esac

export PKG_CONFIG_PATH="$FFMPEG_MPV_DIR/lib/pkgconfig:$GROK_LIBRARY_DIRECTORY/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export DYLD_LIBRARY_PATH="$FFMPEG_MPV_DIR/lib:$GROK_LIBRARY_DIRECTORY${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"

cargo build --release -p imfwizard-cli --manifest-path "$ROOT/rust/Cargo.toml"
"$ROOT/scripts/setup-tauri-bin.sh"
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k.1.dylib" "$ROOT/gui/src-tauri/libgrokj2k.1.dylib"
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k_plugin.dylib" "$ROOT/gui/src-tauri/libgrokj2k_plugin.dylib"
TARGET_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
STAGED_HOMEBREW_LIBRARIES="$ROOT/gui/src-tauri/homebrew-libraries"
rm -rf "$STAGED_HOMEBREW_LIBRARIES"
"$ROOT/scripts/stage-macos-homebrew-libraries.sh" "$ROOT/gui/src-tauri/imfwizard-$TARGET_TRIPLE" "$STAGED_HOMEBREW_LIBRARIES"

FFMPEG_MPV_FILES="$("$ROOT/scripts/ffmpeg-mpv-bundle-config.sh" "$FFMPEG_MPV_DIR" Darwin "$STAGED_HOMEBREW_LIBRARIES")"
PRODUCT_NAME="$(jq -r .productName "$ROOT/gui/src-tauri/tauri.conf.json")"
VERSION="$(jq -r .version "$ROOT/gui/src-tauri/tauri.conf.json")"

cd "$ROOT/gui"
pnpm install --frozen-lockfile
# grok looks for the plugin beside the executable, not in Frameworks
PLUGIN_FILES='{"bundle":{"macOS":{"files":{"MacOS/libgrokj2k_plugin.dylib":"libgrokj2k_plugin.dylib","MacOS/grok_kernels.metallib":"'"$GROK_LIBRARY_DIRECTORY/grok_kernels.metallib"'"}}}}'
pnpm tauri build --bundles dmg --config "$FFMPEG_MPV_FILES" --config "$PLUGIN_FILES"

DMG_DIRECTORY="$ROOT/gui/src-tauri/target/release/bundle/dmg"
BUILT_DMG="$DMG_DIRECTORY/${PRODUCT_NAME}_${VERSION}_${DMG_ARCH}.dmg"
RENAMED_DMG="$DMG_DIRECTORY/${PRODUCT_NAME// /-}-${VERSION}-metal_${DMG_ARCH}.dmg"
mv "$BUILT_DMG" "$RENAMED_DMG"
echo "wrote $RENAMED_DMG"
