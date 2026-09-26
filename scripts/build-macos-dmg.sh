#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GROK_INSTALL="${1:-}"

if [[ -z "$GROK_INSTALL" ]]; then
    echo "usage: $0 /path/to/grok/install" >&2
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

export PKG_CONFIG_PATH="$GROK_LIBRARY_DIRECTORY/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export DYLD_LIBRARY_PATH="$GROK_LIBRARY_DIRECTORY${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"

cargo build --release -p imfwizard-cli --manifest-path "$ROOT/rust/Cargo.toml"
"$ROOT/scripts/setup-tauri-bin.sh"
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k.1.dylib" "$ROOT/gui/src-tauri/libgrokj2k.1.dylib"
cp -L "$GROK_LIBRARY_DIRECTORY/libgrokj2k_plugin.dylib" "$ROOT/gui/src-tauri/libgrokj2k_plugin.dylib"

PRODUCT_NAME="$(jq -r .productName "$ROOT/gui/src-tauri/tauri.conf.json")"
VERSION="$(jq -r .version "$ROOT/gui/src-tauri/tauri.conf.json")"

cd "$ROOT/gui"
pnpm install --frozen-lockfile
# grok looks for the plugin beside the executable, not in Frameworks
PLUGIN_FILES='{"bundle":{"macOS":{"files":{"MacOS/libgrokj2k_plugin.dylib":"libgrokj2k_plugin.dylib","MacOS/grok_kernels.metallib":"'"$GROK_LIBRARY_DIRECTORY/grok_kernels.metallib"'"}}}}'
pnpm tauri build --bundles dmg --config "$PLUGIN_FILES"

DMG_DIRECTORY="$ROOT/gui/src-tauri/target/release/bundle/dmg"
BUILT_DMG="$DMG_DIRECTORY/${PRODUCT_NAME}_${VERSION}_${DMG_ARCH}.dmg"
RENAMED_DMG="$DMG_DIRECTORY/${PRODUCT_NAME// /-}-${VERSION}-metal_${DMG_ARCH}.dmg"
mv "$BUILT_DMG" "$RENAMED_DMG"
echo "wrote $RENAMED_DMG"
