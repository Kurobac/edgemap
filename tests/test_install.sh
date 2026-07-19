#!/usr/bin/env bash
set -Eeuo pipefail

readonly PROJECT_ROOT=$(cd -- "$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")/.." && pwd -P)

if (($# > 1)); then
    printf 'Usage: %s [BINARY_DIR]\n' "${0##*/}" >&2
    exit 1
fi

BINARY_DIR=${1:-target/debug}
if [[ $BINARY_DIR != /* ]]; then
    BINARY_DIR=$PROJECT_ROOT/$BINARY_DIR
fi

TEST_ROOT=$(mktemp -d)
trap 'rm -rf "$TEST_ROOT"' EXIT

PAYLOAD=$TEST_ROOT/payload
DESTINATION=$TEST_ROOT/destination

fail() {
    printf 'install test failed: %s\n' "$*" >&2
    exit 1
}

assert_file() {
    [[ -f $1 ]] || fail "missing file: $1"
}

assert_executable() {
    [[ -x $1 ]] || fail "missing executable: $1"
}

assert_mode() {
    local actual
    actual=$(stat -c '%a' "$2")
    [[ $actual == "$1" ]] || fail "mode $actual != $1: $2"
}

assert_same() {
    cmp -s "$1" "$2" || fail "content mismatch: $2"
}

assert_missing() {
    [[ ! -e $1 ]] || fail "unexpected installed file: $1"
}

stage_and_install() {
    mkdir -p "$DESTINATION/usr/local/lib"
    printf 'keep\n' >"$DESTINATION/usr/local/lib/keep-me"
    mkdir -p "$DESTINATION/usr/local/lib/edgemap-gui/edgemap_gui"
    printf 'stale\n' > \
        "$DESTINATION/usr/local/lib/edgemap-gui/edgemap_gui/stale.py"

    (cd /tmp && "$PROJECT_ROOT/scripts/stage_release.sh" "$PAYLOAD" "$BINARY_DIR")
    (cd /tmp && DESTDIR="$DESTINATION" "$PAYLOAD/install.sh")
}

verify_installation() {
    local source
    local relative

    assert_executable "$DESTINATION/usr/local/bin/dseuhid"
    assert_executable "$DESTINATION/usr/local/bin/edgemap"
    assert_executable "$DESTINATION/usr/local/bin/edgemap-gui"
    assert_mode 755 "$DESTINATION/usr/local/bin/edgemap-gui"
    assert_mode 644 "$DESTINATION/usr/lib/systemd/system/dseuhid.service"
    assert_mode 644 "$DESTINATION/usr/lib/systemd/user/edgemap.service"

    assert_file "$DESTINATION/usr/share/applications/edgemap.desktop"
    assert_file \
        "$DESTINATION/usr/share/icons/hicolor/scalable/apps/edgemap.svg"
    assert_file "$DESTINATION/usr/share/zsh/site-functions/_dseuhid"
    assert_file "$DESTINATION/usr/share/zsh/site-functions/_edgemap"

    [[ ! -e $DESTINATION/usr/local/lib/edgemap-gui/edgemap_gui/stale.py ]] ||
        fail "stale GUI module survived the directory replacement"
    [[ -z $(find "$PAYLOAD/usr/local/lib/edgemap-gui" \
        -type d -name '__pycache__' -print -quit) ]] ||
        fail "release payload contains __pycache__"

    while IFS= read -r -d '' source; do
        relative=${source#"$PROJECT_ROOT/gui/"}
        assert_same "$source" \
            "$DESTINATION/usr/local/lib/edgemap-gui/$relative"
    done < <(
        find "$PROJECT_ROOT/gui/edgemap_gui" -type f -name '*.py' -print0 |
            sort -z
    )

    assert_same "$PROJECT_ROOT/gui/edgemap-gui" \
        "$DESTINATION/usr/local/bin/edgemap-gui"
    grep -q '/usr/local/bin/dseuhid' \
        "$DESTINATION/usr/lib/systemd/system/dseuhid.service" ||
        fail "system service does not use the release binary prefix"
    grep -q '/usr/local/bin/edgemap' \
        "$DESTINATION/usr/lib/systemd/user/edgemap.service" ||
        fail "user service does not use the release binary prefix"
}

verify_safety_guards() {
    local output=$TEST_ROOT/safety-error

    if "$PROJECT_ROOT/scripts/stage_release.sh" "$PAYLOAD" "$BINARY_DIR" \
        >"$output" 2>&1; then
        fail "release staging overwrote an existing output directory"
    fi
    grep -q 'OUTPUT_DIR already exists' "$output" ||
        fail "release staging did not explain the overwrite refusal"

    if DESTDIR=/ "$PAYLOAD/install.sh" >"$output" 2>&1; then
        fail "installer accepted DESTDIR=/"
    fi
    grep -q 'DESTDIR=/ is not a staging directory' "$output" ||
        fail "installer did not explain the unsafe DESTDIR"

    if DESTDIR=// "$PAYLOAD/install.sh" >"$output" 2>&1; then
        fail "installer accepted a root-equivalent DESTDIR"
    fi
    grep -q 'DESTDIR=/ is not a staging directory' "$output" ||
        fail "installer did not reject a root-equivalent DESTDIR"

    if ((EUID != 0)); then
        if "$PAYLOAD/install.sh" >"$output" 2>&1; then
            fail "unprivileged installer wrote to the real root"
        fi
        grep -q 'changes to / require root' "$output" ||
            fail "installer did not explain the root requirement"
    fi

    if DESTDIR="$DESTINATION" "$PAYLOAD/install.sh" remove \
        >"$output" 2>&1; then
        fail "installer accepted an unknown action"
    fi
    grep -q 'Usage: install.sh \[uninstall\]' "$output" ||
        fail "installer did not explain the supported uninstall action"
    assert_executable "$DESTINATION/usr/local/bin/dseuhid"
}

verify_uninstallation() {
    local path

    (cd /tmp && DESTDIR="$DESTINATION" "$PAYLOAD/install.sh" uninstall)

    for path in \
        usr/local/bin/dseuhid \
        usr/local/bin/edgemap \
        usr/local/bin/edgemap-gui \
        usr/local/lib/edgemap-gui \
        usr/lib/systemd/system/dseuhid.service \
        usr/lib/systemd/user/edgemap.service \
        usr/share/applications/edgemap.desktop \
        usr/share/icons/hicolor/scalable/apps/edgemap.svg \
        usr/share/zsh/site-functions/_dseuhid \
        usr/share/zsh/site-functions/_edgemap; do
        assert_missing "$DESTINATION/$path"
    done

    assert_file "$DESTINATION/usr/local/lib/keep-me"
    [[ -d $DESTINATION/usr/local/bin ]] ||
        fail "uninstall removed a shared parent directory"
}

verify_preflight_failure() {
    local incomplete_payload=$TEST_ROOT/incomplete-payload
    local failed_destination=$TEST_ROOT/failed-destination
    local output=$TEST_ROOT/preflight-error

    cp -a "$PAYLOAD" "$incomplete_payload"
    rm "$incomplete_payload/usr/share/zsh/site-functions/_edgemap"
    mkdir -p "$failed_destination"
    printf 'untouched\n' >"$failed_destination/sentinel"

    if DESTDIR="$failed_destination" "$incomplete_payload/install.sh" >"$output" 2>&1; then
        fail "installer accepted an incomplete release payload"
    fi
    grep -q 'release payload is missing: usr/share/zsh/site-functions/_edgemap' \
        "$output" || fail "installer did not identify the missing payload file"
    [[ $(<"$failed_destination/sentinel") == untouched ]] ||
        fail "failed preflight modified the staging root"
    [[ ! -e $failed_destination/usr/local/bin/dseuhid ]] ||
        fail "failed preflight installed files"
}

stage_and_install
verify_installation
verify_safety_guards
verify_uninstallation
verify_preflight_failure

printf 'install integration test passed\n'
