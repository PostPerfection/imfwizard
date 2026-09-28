#!/usr/bin/env bash

set -euo pipefail

if [[ $# -lt 3 ]]
then
    echo "usage: $0 <binary> <library directory> <search directory>..." >&2
    exit 2
fi

BINARY="$1"
LIBRARY_DIRECTORY="$2"
shift 2
SEARCH_DIRECTORIES=("$@")

load_commands() {
    local own_id
    own_id="$(otool -D "$1" | tail -n +2)"
    # otool -L lists a dylib's own id among its references
    otool -L "$1" | awk -v own_id="$own_id" 'NR > 1 && $1 != own_id {print $1}'
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
    for directory in "$referrer_directory" "${SEARCH_DIRECTORIES[@]}"
    do
        if [[ -f "$directory/$name" ]]
        then
            echo "$directory/$name"
            return
        fi
    done
    echo "bundle-macos-cli-libraries: no file for ${reference} under ${SEARCH_DIRECTORIES[*]}" >&2
    exit 1
}

mkdir -p "$LIBRARY_DIRECTORY"
install_name_tool -add_rpath @loader_path/lib "$BINARY"

queue=("$BINARY")
queue_source_directories=("$(dirname "$BINARY")")
index=0
while [[ $index -lt ${#queue[@]} ]]
do
    file="${queue[$index]}"
    source_directory="${queue_source_directories[$index]}"
    index=$((index + 1))
    references="$(load_commands "$file")"
    for reference in $references
    do
        if [[ "$reference" == /usr/lib/* || "$reference" == /System/* ]]
        then
            continue
        fi
        name="$(basename "$reference")"
        bundled="$LIBRARY_DIRECTORY/$name"
        if [[ ! -f "$bundled" ]]
        then
            source_file="$(library_source "$reference" "$source_directory")"
            cp -L "$source_file" "$bundled"
            # homebrew installs its dylibs read only
            chmod 644 "$bundled"
            install_name_tool -id "@rpath/$name" "$bundled"
            queue+=("$bundled")
            queue_source_directories+=("$(dirname "$source_file")")
        fi
        if [[ "$file" == "$BINARY" ]]
        then
            install_name_tool -change "$reference" "@rpath/$name" "$file"
        else
            install_name_tool -change "$reference" "@loader_path/$name" "$file"
        fi
    done
    # every install_name_tool edit breaks the signature arm64 requires
    codesign --force --sign - "$file"
done

echo "bundle-macos-cli-libraries: $BINARY loads $((${#queue[@]} - 1)) libraries from $LIBRARY_DIRECTORY"
