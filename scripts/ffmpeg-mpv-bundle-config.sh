#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCHIVE_ROOT="${1:-${FFMPEG_MPV_DIR:-}}"
PLATFORM="${2:-$(uname -s)}"
STAGED_HOMEBREW_LIBRARIES="${3:-${STAGED_HOMEBREW_LIBRARIES:-}}"

if [[ -z "$ARCHIVE_ROOT" ]]
then
    echo "usage: $0 [ffmpeg-mpv archive root, default \$FFMPEG_MPV_DIR] [Linux or Darwin, default uname -s] [Darwin only: staged Homebrew library directory, default \$STAGED_HOMEBREW_LIBRARIES]" >&2
    exit 2
fi

LIBRARY_DIRECTORY="$ARCHIVE_ROOT/lib"
LICENCE_SOURCE="$ARCHIVE_ROOT/THIRD-PARTY-LICENSES"
LICENCE_NAME="THIRD-PARTY-LICENSES-ffmpeg-mpv.txt"
UBUNTU_RUNTIME_PACKAGES="$ARCHIVE_ROOT/ubuntu-24.04-runtime-packages.txt"
LINUX_CONFIG="$ROOT/gui/src-tauri/tauri.linux.conf.json"
MACOS_CONFIG="$ROOT/gui/src-tauri/tauri.macos.conf.json"
INSTALLED_LIBRARY_DIRECTORY="/usr/lib/imfwizard"
INSTALLED_LICENCE="/usr/share/doc/imfwizard/$LICENCE_NAME"
# glibc, libgcc and libstdc++ come with every Fedora install
IMPLIED_SONAMES="ld-linux-x86-64.so.2 libc.so.6 libm.so.6 libgcc_s.so.1 libstdc++.so.6"
RPM_SONAME_SUFFIX="()(64bit)"

for required in "$LIBRARY_DIRECTORY" "$LICENCE_SOURCE"
do
    if [[ ! -e "$required" ]]
    then
        echo "ffmpeg-mpv-bundle-config: $required does not exist" >&2
        exit 1
    fi
done

elf_entries() {
    readelf -d "$2" | sed -n "s/.*($1).*\[\(.*\)\]/\1/p"
}

json_array() {
    jq -R . | jq -s -c .
}

linux_config() {
    local sonames=() needed=() library
    while IFS= read -r library
    do
        sonames+=("$(elf_entries SONAME "$library")")
        mapfile -t -O "${#needed[@]}" needed < <(elf_entries NEEDED "$library")
    done < <(find "$LIBRARY_DIRECTORY" -maxdepth 1 -type f -name '*.so.*' | sort)

    local files
    files="$(printf '%s\n' "${sonames[@]}" | jq -R -n -c \
        --arg source "$LIBRARY_DIRECTORY" \
        --arg destination "$INSTALLED_LIBRARY_DIRECTORY" \
        --arg licence_source "$LICENCE_SOURCE" \
        --arg licence_destination "$INSTALLED_LICENCE" \
        '[inputs | {key: ($destination + "/" + .), value: ($source + "/" + .)}] | from_entries
            + {($licence_destination): $licence_source}')"

    local deb_packages rpm_requires
    deb_packages="$(tr -s '[:space:]' '\n' < "$UBUNTU_RUNTIME_PACKAGES" | sed '/^$/d' | json_array)"
    rpm_requires="$(printf '%s\n' "${needed[@]}" | sort -u \
        | grep -vxF -f <(printf '%s\n' "${sonames[@]}" $IMPLIED_SONAMES) \
        | sed "s/\$/$RPM_SONAME_SUFFIX/" | json_array)"

    # --config replaces arrays, so the static depends are carried over
    jq -c \
        --argjson files "$files" \
        --argjson deb_packages "$deb_packages" \
        --argjson rpm_requires "$rpm_requires" \
        '{bundle: {linux: {
            deb: {depends: (.bundle.linux.deb.depends + $deb_packages), files: $files},
            rpm: {depends: (.bundle.linux.rpm.depends + $rpm_requires), files: $files}
        }}}' "$LINUX_CONFIG"
}

macos_config() {
    if [[ ! -d "$STAGED_HOMEBREW_LIBRARIES" ]]
    then
        echo "ffmpeg-mpv-bundle-config: the staged Homebrew library directory '$STAGED_HOMEBREW_LIBRARIES' does not exist" >&2
        exit 1
    fi
    local otool library
    otool="$(command -v otool || command -v llvm-otool)"
    local frameworks=()
    while IFS= read -r library
    do
        frameworks+=("$LIBRARY_DIRECTORY/$(basename "$("$otool" -D "$library" | tail -n +2)")")
    done < <(find "$LIBRARY_DIRECTORY" -maxdepth 1 -type f -name '*.dylib' | sort)
    # the staging script names each copy after its id
    while IFS= read -r library
    do
        frameworks+=("$library")
    done < <(find "$STAGED_HOMEBREW_LIBRARIES" -maxdepth 1 -type f -name '*.dylib' | sort)

    # a bundle.resources map would replace tauri.conf.json's photon list
    jq -c \
        --argjson frameworks "$(printf '%s\n' "${frameworks[@]}" | json_array)" \
        --arg licence_source "$LICENCE_SOURCE" \
        --arg licence_destination "Resources/$LICENCE_NAME" \
        '{bundle: {macOS: {
            frameworks: (.bundle.macOS.frameworks + $frameworks),
            files: {($licence_destination): $licence_source}
        }}}' "$MACOS_CONFIG"
}

if [[ "$PLATFORM" == Linux ]]
then
    linux_config
elif [[ "$PLATFORM" == Darwin ]]
then
    macos_config
else
    echo "ffmpeg-mpv-bundle-config: no bundle config for $PLATFORM" >&2
    exit 1
fi
