#!/usr/bin/env bash
set -Eeuo pipefail

PROJECT_ROOT=$(cd -- "$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")/.." && pwd -P)
readonly PROJECT_ROOT

if (($# > 1)); then
    printf 'Usage: %s [BINARY_DIR]\n' "${0##*/}" >&2
    exit 1
fi
if ((EUID != 0)) || [[ ${CI:-} != true ]]; then
    printf 'error: the fixed-path install test requires root in CI=true\n' >&2
    exit 1
fi

BINARY_DIR=${1:-target/debug}
if [[ $BINARY_DIR != /* ]]; then
    BINARY_DIR=$PROJECT_ROOT/$BINARY_DIR
fi

unset DESTDIR

TEST_ROOT=$(mktemp -d)
readonly TEST_ROOT
PAYLOAD=$TEST_ROOT/payload
SYSTEM_CLEANUP_ARMED=false

SYSTEM_TARGETS=(
    /usr/local/bin/dseuhid
    /usr/local/bin/edgemap
    /usr/local/bin/edgemap-gui
    /usr/local/lib/edgemap-gui
    /usr/lib/systemd/system/dseuhid.service
    /usr/lib/systemd/user/edgemap.service
    /usr/share/applications/edgemap.desktop
    /usr/share/icons/hicolor/scalable/apps/edgemap.svg
    /usr/share/zsh/site-functions/_dseuhid
    /usr/share/zsh/site-functions/_edgemap
)

cleanup() {
    local status=$?
    local cleanup_status=0
    local temp_status=0

    trap - EXIT
    set +e
    if [[ $SYSTEM_CLEANUP_ARMED == true && -x $PAYLOAD/install.sh ]]; then
        "$PAYLOAD/install.sh" uninstall >/dev/null 2>&1
        cleanup_status=$?
    fi
    rm -rf -- "$TEST_ROOT"
    temp_status=$?

    if ((status == 0 && cleanup_status != 0)); then
        status=$cleanup_status
    fi
    if ((status == 0 && temp_status != 0)); then
        status=$temp_status
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

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
    [[ ! -e $1 && ! -L $1 ]] || fail "unexpected installed file: $1"
}

assert_system_targets_absent() {
    local path

    for path in "${SYSTEM_TARGETS[@]}"; do
        assert_missing "$path"
    done
}

stage_payload() {
    (cd /tmp && "$PROJECT_ROOT/scripts/stage_release.sh" "$PAYLOAD" "$BINARY_DIR")
}

verify_release_payload() {
    local relative
    local source

    assert_executable "$PAYLOAD/dseuhid"
    assert_executable "$PAYLOAD/edgemap"
    assert_executable "$PAYLOAD/edgemap-gui"
    assert_executable "$PAYLOAD/install.sh"
    assert_same "$PROJECT_ROOT/LICENSE" "$PAYLOAD/LICENSE"
    assert_mode 644 "$PAYLOAD/usr/lib/systemd/system/dseuhid.service"
    assert_mode 644 "$PAYLOAD/usr/lib/systemd/user/edgemap.service"
    assert_file "$PAYLOAD/usr/share/applications/edgemap.desktop"
    assert_file "$PAYLOAD/usr/share/icons/hicolor/scalable/apps/edgemap.svg"
    assert_file "$PAYLOAD/usr/share/zsh/site-functions/_dseuhid"
    assert_file "$PAYLOAD/usr/share/zsh/site-functions/_edgemap"

    while IFS= read -r -d '' source; do
        relative=${source#"$PROJECT_ROOT/gui/"}
        assert_same "$source" "$PAYLOAD/usr/local/lib/edgemap-gui/$relative"
    done < <(
        find "$PROJECT_ROOT/gui/edgemap_gui" -type f -name '*.py' -print0 |
            sort -z
    )

    assert_same "$PROJECT_ROOT/gui/edgemap-gui" "$PAYLOAD/edgemap-gui"
    assert_same "$PROJECT_ROOT/install.sh" "$PAYLOAD/install.sh"
    [[ -z $(find "$PAYLOAD/usr/local/lib/edgemap-gui" \
        -type d -name '__pycache__' -print -quit) ]] ||
        fail "release payload contains __pycache__"
    grep -q '/usr/local/bin/dseuhid' \
        "$PAYLOAD/usr/lib/systemd/system/dseuhid.service" ||
        fail "system service does not use the release binary prefix"
    grep -q '/usr/local/bin/edgemap' \
        "$PAYLOAD/usr/lib/systemd/user/edgemap.service" ||
        fail "user service does not use the release binary prefix"
}

verify_release_safety() {
    local alias_output=$TEST_ROOT/alias-error
    local fail_output=$TEST_ROOT/failure-error
    local fail_tools=$TEST_ROOT/fail-tools
    local output=$TEST_ROOT/safety-error
    local victim=$TEST_ROOT/victim

    if "$PROJECT_ROOT/scripts/stage_release.sh" "$PAYLOAD" "$BINARY_DIR" \
        >"$output" 2>&1; then
        fail "release staging overwrote an existing output directory"
    fi
    grep -q 'OUTPUT_DIR already exists' "$output" ||
        fail "release staging did not explain the overwrite refusal"

    mkdir "$victim"
    printf 'keep\n' >"$victim/sentinel"
    if "$PROJECT_ROOT/scripts/stage_release.sh" \
        "$TEST_ROOT/missing/../victim" "$BINARY_DIR" \
        >"$alias_output" 2>&1; then
        fail "release staging accepted an alias of an existing output directory"
    fi
    grep -q 'OUTPUT_DIR already exists' "$alias_output" ||
        fail "release staging did not reject the resolved existing directory"
    [[ $(<"$victim/sentinel") == keep ]] ||
        fail "release staging modified the existing output sentinel"
    assert_missing "$victim/dseuhid"

    mkdir "$fail_tools"
    printf '#!/usr/bin/env bash\nexit 73\n' >"$fail_tools/install"
    chmod 755 "$fail_tools/install"
    printf 'keep\n' >"$TEST_ROOT/adjacent-sentinel"
    if PATH="$fail_tools:$PATH" "$PROJECT_ROOT/scripts/stage_release.sh" \
        "$TEST_ROOT/owned-failure" "$BINARY_DIR" \
        >"$fail_output" 2>&1; then
        fail "release staging ignored an injected staging failure"
    fi
    assert_missing "$TEST_ROOT/owned-failure"
    [[ $(<"$TEST_ROOT/adjacent-sentinel") == keep ]] ||
        fail "release cleanup modified an adjacent sentinel"
}

verify_missing_source_preflight() {
    local incomplete_binary_dir=$TEST_ROOT/incomplete-binaries
    local output=$TEST_ROOT/missing-source-error
    local release_dir=$TEST_ROOT/missing-source-release

    mkdir "$incomplete_binary_dir"
    cp "$BINARY_DIR/dseuhid" "$incomplete_binary_dir/dseuhid"
    if "$PROJECT_ROOT/scripts/stage_release.sh" "$release_dir" \
        "$incomplete_binary_dir" >"$output" 2>&1; then
        fail "release staging accepted an incomplete source payload"
    fi
    grep -q 'required release source is missing' "$output" ||
        fail "release staging did not explain the missing source payload"
    assert_missing "$release_dir"
}

verify_installer_guards() {
    local output=$TEST_ROOT/installer-error

    if "$PAYLOAD/install.sh" remove >"$output" 2>&1; then
        fail "installer accepted an unknown action"
    fi
    grep -q 'Usage: install.sh \[uninstall\]' "$output" ||
        fail "installer did not explain the supported uninstall action"
    assert_system_targets_absent

    if "$PAYLOAD/install.sh" uninstall extra >"$output" 2>&1; then
        fail "installer accepted extra arguments"
    fi
    grep -q 'Usage: install.sh \[uninstall\]' "$output" ||
        fail "installer did not reject extra arguments"
    assert_system_targets_absent

    if DESTDIR="$TEST_ROOT/destination" "$PAYLOAD/install.sh" \
        >"$output" 2>&1; then
        fail "installer accepted the removed DESTDIR mode"
    fi
    grep -q 'DESTDIR is not supported' "$output" ||
        fail "installer did not explain that DESTDIR was removed"
    assert_system_targets_absent
}

verify_preflight_failure() {
    local incomplete_payload=$TEST_ROOT/incomplete-payload
    local output=$TEST_ROOT/preflight-error

    cp -a "$PAYLOAD" "$incomplete_payload"
    rm "$incomplete_payload/usr/share/zsh/site-functions/_edgemap"

    if "$incomplete_payload/install.sh" >"$output" 2>&1; then
        fail "installer accepted an incomplete release payload"
    fi
    grep -q 'release payload is missing: usr/share/zsh/site-functions/_edgemap' \
        "$output" || fail "installer did not identify the missing payload file"
    assert_system_targets_absent
}

verify_installation() {
    local relative
    local source

    assert_same "$PAYLOAD/dseuhid" /usr/local/bin/dseuhid
    assert_same "$PAYLOAD/edgemap" /usr/local/bin/edgemap
    assert_same "$PAYLOAD/edgemap-gui" /usr/local/bin/edgemap-gui
    assert_mode 755 /usr/local/bin/dseuhid
    assert_mode 755 /usr/local/bin/edgemap
    assert_mode 755 /usr/local/bin/edgemap-gui
    assert_mode 644 /usr/lib/systemd/system/dseuhid.service
    assert_mode 644 /usr/lib/systemd/user/edgemap.service
    assert_file /usr/share/applications/edgemap.desktop
    assert_file /usr/share/icons/hicolor/scalable/apps/edgemap.svg
    assert_file /usr/share/zsh/site-functions/_dseuhid
    assert_file /usr/share/zsh/site-functions/_edgemap

    while IFS= read -r -d '' source; do
        relative=${source#"$PROJECT_ROOT/gui/"}
        assert_same "$source" "/usr/local/lib/edgemap-gui/$relative"
    done < <(
        find "$PROJECT_ROOT/gui/edgemap_gui" -type f -name '*.py' -print0 |
            sort -z
    )

    grep -q '/usr/local/bin/dseuhid' \
        /usr/lib/systemd/system/dseuhid.service ||
        fail "installed system service does not use the release binary prefix"
    grep -q '/usr/local/bin/edgemap' \
        /usr/lib/systemd/user/edgemap.service ||
        fail "installed user service does not use the release binary prefix"
}

verify_upgrade_replaces_gui() {
    printf 'stale\n' >/usr/local/lib/edgemap-gui/edgemap_gui/stale.py
    (cd /tmp && "$PAYLOAD/install.sh")
    assert_missing /usr/local/lib/edgemap-gui/edgemap_gui/stale.py
    verify_installation
}

verify_uninstallation() {
    (cd /tmp && "$PAYLOAD/install.sh" uninstall)
    assert_system_targets_absent
}

stage_payload
verify_release_payload
verify_release_safety
verify_missing_source_preflight

assert_system_targets_absent
SYSTEM_CLEANUP_ARMED=true
verify_installer_guards
verify_preflight_failure

(cd /tmp && "$PAYLOAD/install.sh")
verify_installation
verify_upgrade_replaces_gui
verify_uninstallation
SYSTEM_CLEANUP_ARMED=false

printf 'release and fixed-path install integration test passed\n'
