#!/usr/bin/env bash
set -Eeuo pipefail

readonly SCRIPT_NAME=${0##*/}
PROJECT_ROOT=$(cd -- "$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")/.." && pwd -P)
readonly PROJECT_ROOT

if (($# != 1)); then
    printf 'Usage: %s <TAG>\n' "$SCRIPT_NAME" >&2
    exit 1
fi

package_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -n 1)
if [[ -z $package_version ]]; then
    printf 'error: failed to read package version from Cargo.toml\n' >&2
    exit 1
fi

expected_tag=v$package_version
if [[ $1 != "$expected_tag" ]]; then
    printf 'error: release tag %s does not match package version %s\n' "$1" "$expected_tag" >&2
    exit 1
fi

printf 'Release tag matches package version: %s\n' "$expected_tag"
