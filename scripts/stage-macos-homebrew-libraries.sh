#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 2 ]]
then
    echo "usage: $0 <binary> <staging directory>" >&2
    exit 2
fi

BINARY="$1"
STAGING_DIRECTORY="$2"
SYSTEM_REFERENCE='^(/usr/lib/|/System/)'
BUNDLE_REFERENCE='^(@rpath/|@executable_path/|@loader_path/)'
# the relink step points grok at the copy bundle.macOS.frameworks names
GROK_REFERENCE='(^|/)libgrokj2k[^/]*$'

load_commands() {
    local own_id
    own_id="$(otool -D "$1" | tail -n +2)"
    # otool -L lists a dylib's own id among its references
    otool -L "$1" | awk -v own_id="$own_id" 'NR > 1 && $1 != own_id {print $1}'
}

outside_references() {
    if [[ "$1" == "$BINARY" ]]
    then
        load_commands "$1" | grep -vE "$SYSTEM_REFERENCE|$BUNDLE_REFERENCE|$GROK_REFERENCE" || true
    else
        load_commands "$1" | grep -vE "$SYSTEM_REFERENCE" || true
    fi
}

# a relative reference like @loader_path/libicudata.77.dylib sits beside the library that loads it
library_source() {
    local reference="$1"
    local referrer_directory="$2"
    local name
    name="$(basename "$reference")"
    if [[ "$reference" == /* && -f "$reference" ]]
    then
        echo "$reference"
        return
    fi
    if [[ -f "$referrer_directory/$name" ]]
    then
        echo "$referrer_directory/$name"
        return
    fi
    echo "stage-macos-homebrew-libraries: no file for ${reference} under ${referrer_directory}" >&2
    exit 1
}

mkdir -p "$STAGING_DIRECTORY"

queue=("$BINARY")
queue_source_directories=("$(dirname "$BINARY")")
index=0
while [[ $index -lt ${#queue[@]} ]]
do
    file="${queue[$index]}"
    source_directory="${queue_source_directories[$index]}"
    index=$((index + 1))
    references="$(outside_references "$file")"
    for reference in $references
    do
        name="$(basename "$reference")"
        staged="$STAGING_DIRECTORY/$name"
        if [[ ! -f "$staged" ]]
        then
            source_file="$(library_source "$reference" "$source_directory")"
            cp -L "$source_file" "$staged"
            # homebrew installs its dylibs read only
            chmod 644 "$staged"
            install_name_tool -id "@rpath/$name" "$staged"
            queue+=("$staged")
            queue_source_directories+=("$(dirname "$source_file")")
        fi
        if [[ "$file" != "$BINARY" ]]
        then
            install_name_tool -change "$reference" "@loader_path/$name" "$file"
        fi
    done
    if [[ "$file" != "$BINARY" ]]
    then
        # every install_name_tool edit breaks the signature arm64 requires
        codesign --force --sign - "$file"
    fi
done

echo "stage-macos-homebrew-libraries: $BINARY loads $((${#queue[@]} - 1)) libraries staged in $STAGING_DIRECTORY"
