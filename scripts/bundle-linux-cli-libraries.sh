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

needed_sonames() {
    readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'
}

# a soname found in no search directory is a system library
library_source() {
    local directory
    for directory in "${SEARCH_DIRECTORIES[@]}"
    do
        if [[ -f "$directory/$1" ]]
        then
            echo "$directory/$1"
            return
        fi
    done
}

mkdir -p "$LIBRARY_DIRECTORY"
declare -A bundled_sonames=()
queue=("$BINARY")
index=0
while [[ $index -lt ${#queue[@]} ]]
do
    file="${queue[$index]}"
    index=$((index + 1))
    for soname in $(needed_sonames "$file")
    do
        if [[ -n "${bundled_sonames[$soname]:-}" ]]
        then
            continue
        fi
        source_file="$(library_source "$soname")"
        if [[ -z "$source_file" ]]
        then
            continue
        fi
        cp -L "$source_file" "$LIBRARY_DIRECTORY/$soname"
        bundled_sonames[$soname]=1
        queue+=("$LIBRARY_DIRECTORY/$soname")
    done
done

echo "bundle-linux-cli-libraries: $BINARY loads ${#bundled_sonames[@]} libraries from $LIBRARY_DIRECTORY: ${!bundled_sonames[*]}"
