#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAURI_DIR="${ROOT}/gui/src-tauri"
TARGET_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
DYLIB_NAME="libgrokj2k.1.dylib"
INSTALL_NAME="@executable_path/../Frameworks/${DYLIB_NAME}"

if [[ "${TAURI_ENV_DEBUG:-false}" == "true" ]]; then
    PROFILE_DIR="debug"
else
    PROFILE_DIR="release"
fi

STAGED_DYLIB="${TAURI_DIR}/${DYLIB_NAME}"
FILES=(
    "${STAGED_DYLIB}"
    "${TAURI_DIR}/target/${PROFILE_DIR}/imfwizard-gui"
    "${TAURI_DIR}/imfwizard-${TARGET_TRIPLE}"
)

for file in "${FILES[@]}"; do
    if [[ ! -f "${file}" ]]; then
        echo "relink-macos-grok: ${file} does not exist" >&2
        exit 1
    fi
done

# a binary linked against this dylib records whatever its id says
install_name_tool -id "${INSTALL_NAME}" "${STAGED_DYLIB}"

for file in "${FILES[@]}"; do
    for reference in $(otool -L "${file}" | awk 'NR > 1 && /libgrok/ {print $1}'); do
        case "$(basename "${reference}")" in
            "${DYLIB_NAME}")
                install_name_tool -change "${reference}" "${INSTALL_NAME}" "${file}"
                ;;
            *)
                echo "relink-macos-grok: ${file} loads ${reference}, which the bundle does not carry" >&2
                exit 1
                ;;
        esac
    done
    # every install_name_tool edit breaks the signature arm64 requires
    codesign --force --sign - "${file}"
done

for file in "${FILES[@]}"; do
    if otool -L "${file}" | awk 'NR > 1 && /libgrok/ {print $1}' | grep -qv "^@executable_path/"; then
        otool -L "${file}" >&2
        echo "relink-macos-grok: ${file} still loads libgrokj2k from outside the bundle" >&2
        exit 1
    fi
done

echo "relink-macos-grok: the gui and its sidecar load ${INSTALL_NAME}"
