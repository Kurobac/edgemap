#!/usr/bin/env bash
set -Eeuo pipefail

PROJECT_ROOT=$(cd -- "$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")/.." && pwd -P)
readonly PROJECT_ROOT

package_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -n 1)
[[ -n $package_version ]] || {
    printf 'release tag test failed: package version not found\n' >&2
    exit 1
}

bash "$PROJECT_ROOT/scripts/verify_release_tag.sh" "v$package_version" >/dev/null

output=$(mktemp)
trap 'rm -f -- "$output"' EXIT
if bash "$PROJECT_ROOT/scripts/verify_release_tag.sh" "v$package_version-mismatch" \
    >"$output" 2>&1; then
    printf 'release tag test failed: mismatched tag was accepted\n' >&2
    exit 1
fi
grep -q 'does not match package version' "$output" || {
    printf 'release tag test failed: mismatch error was not explained\n' >&2
    exit 1
}

printf 'release tag validation test passed\n'
