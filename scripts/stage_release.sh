#!/usr/bin/env bash
set -Eeuo pipefail

readonly SCRIPT_NAME=${0##*/}
readonly PROJECT_ROOT=$(cd -- "$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")/.." && pwd -P)

OUTPUT_DIR=""
STAGING_COMPLETE=false

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Usage: $SCRIPT_NAME <OUTPUT_DIR> <BINARY_DIR>

Create an unpacked edgemap release tree from repository sources and compiled
dseuhid/edgemap binaries. OUTPUT_DIR must not already exist.
EOF
}

cleanup() {
    if [[ $STAGING_COMPLETE == false && -n $OUTPUT_DIR && -d $OUTPUT_DIR ]]; then
        rm -rf -- "$OUTPUT_DIR"
    fi
}
trap cleanup EXIT

require_file() {
    [[ -f $1 ]] || die "required release source is missing: $1"
}

require_executable() {
    require_file "$1"
    [[ -x $1 ]] || die "required release source is not executable: $1"
}

resolve_output_path() {
    local requested=$1
    local parent
    local name

    [[ $requested != */ ]] || requested=${requested%/}
    [[ -n $requested ]] || die "OUTPUT_DIR must not be empty or /"
    [[ ! -e $requested ]] || die "OUTPUT_DIR already exists: $requested"

    parent=$(dirname -- "$requested")
    name=$(basename -- "$requested")
    mkdir -p -- "$parent"
    parent=$(cd -- "$parent" && pwd -P)
    OUTPUT_DIR=$parent/$name
}

validate_sources() {
    local binary_dir=$1
    local required
    local unexpected

    require_executable "$binary_dir/dseuhid"
    require_executable "$binary_dir/edgemap"
    require_executable "$PROJECT_ROOT/gui/edgemap-gui"
    require_executable "$PROJECT_ROOT/install.sh"

    for required in \
        dseuhid.service \
        edgemap.service \
        edgemap.desktop \
        edgemap.svg \
        completions/_dseuhid \
        completions/_edgemap \
        gui/edgemap_gui/__init__.py; do
        require_file "$PROJECT_ROOT/$required"
    done

    unexpected=$(find "$PROJECT_ROOT/gui/edgemap_gui" \
        -type d -name '__pycache__' -prune -o \
        ! -type d ! \( -type f -name '*.py' \) -print -quit)
    [[ -z $unexpected ]] || die "unexpected GUI source entry: $unexpected"
}

stage_python_package() {
    local source
    local relative

    while IFS= read -r -d '' source; do
        relative=${source#"$PROJECT_ROOT/gui/"}
        install -Dm644 "$source" \
            "$OUTPUT_DIR/usr/local/lib/edgemap-gui/$relative"
    done < <(
        find "$PROJECT_ROOT/gui/edgemap_gui" -type f -name '*.py' -print0 | sort -z
    )
}

stage_release() {
    local binary_dir=$1

    install -Dm755 "$binary_dir/dseuhid" "$OUTPUT_DIR/dseuhid"
    install -Dm755 "$binary_dir/edgemap" "$OUTPUT_DIR/edgemap"
    install -Dm755 "$PROJECT_ROOT/gui/edgemap-gui" "$OUTPUT_DIR/edgemap-gui"
    install -Dm755 "$PROJECT_ROOT/install.sh" "$OUTPUT_DIR/install.sh"

    stage_python_package

    sed 's|/usr/bin/|/usr/local/bin/|g' "$PROJECT_ROOT/dseuhid.service" |
        install -Dm644 /dev/stdin \
            "$OUTPUT_DIR/usr/lib/systemd/system/dseuhid.service"
    sed 's|/usr/bin/|/usr/local/bin/|g' "$PROJECT_ROOT/edgemap.service" |
        install -Dm644 /dev/stdin \
            "$OUTPUT_DIR/usr/lib/systemd/user/edgemap.service"

    install -Dm644 "$PROJECT_ROOT/edgemap.desktop" \
        "$OUTPUT_DIR/usr/share/applications/edgemap.desktop"
    install -Dm644 "$PROJECT_ROOT/edgemap.svg" \
        "$OUTPUT_DIR/usr/share/icons/hicolor/scalable/apps/edgemap.svg"
    install -Dm644 "$PROJECT_ROOT/completions/_dseuhid" \
        "$OUTPUT_DIR/usr/share/zsh/site-functions/_dseuhid"
    install -Dm644 "$PROJECT_ROOT/completions/_edgemap" \
        "$OUTPUT_DIR/usr/share/zsh/site-functions/_edgemap"
}

verify_release() {
    local required

    for required in dseuhid edgemap edgemap-gui install.sh; do
        [[ -x $OUTPUT_DIR/$required ]] ||
            die "staged executable verification failed: $required"
    done
    for required in \
        usr/local/lib/edgemap-gui/edgemap_gui/__init__.py \
        usr/lib/systemd/system/dseuhid.service \
        usr/lib/systemd/user/edgemap.service \
        usr/share/applications/edgemap.desktop \
        usr/share/icons/hicolor/scalable/apps/edgemap.svg \
        usr/share/zsh/site-functions/_dseuhid \
        usr/share/zsh/site-functions/_edgemap; do
        [[ -f $OUTPUT_DIR/$required ]] ||
            die "staged file verification failed: $required"
    done

    found=$(find "$OUTPUT_DIR/usr/local/lib/edgemap-gui" \
        -type d -name '__pycache__' -print -quit)
    [[ -z $found ]] || die "staged GUI package contains __pycache__"
}

main() {
    (($# == 2)) || {
        usage >&2
        exit 1
    }

    local binary_dir=$2
    if [[ $binary_dir != /* ]]; then
        binary_dir=$PWD/$binary_dir
    fi

    validate_sources "$binary_dir"
    resolve_output_path "$1"
    stage_release "$binary_dir"
    verify_release
    STAGING_COMPLETE=true
    printf 'Release tree staged: %s\n' "$OUTPUT_DIR"
}

main "$@"
