#!/bin/bash
# Cross-compiles every release target in one container build and writes the
# binaries to packaging/dist/, plus the glibc floor the control file depends on.

set -euo pipefail

if [ $# -gt 0 ]; then
    echo "Usage: $0" >&2
    echo "Builds all targets at once; there is no per-target invocation." >&2
    exit 2
fi

# CONTAINER_CMD picks the runtime where both exist and the default is wrong —
# CI sets it, rootless podman on a hosted runner being the unknown here.
if [ -z "${CONTAINER_CMD:-}" ]; then
    if command -v podman &> /dev/null; then
        CONTAINER_CMD="podman"
    elif command -v docker &> /dev/null; then
        CONTAINER_CMD="docker"
    else
        echo "Error: Neither podman nor docker found" >&2
        exit 1
    fi
fi

cd "$(dirname "$0")"

echo "Using $CONTAINER_CMD as container runtime"
$CONTAINER_CMD build -f Containerfile --output "type=local,dest=dist" ..

BINARIES=(dist/ticpu-claude-command-hook.* dist/gf.*)

# readelf rather than objdump: objdump's target support depends on how the
# binutils at hand was configured, which differs between builder and laptop.
floor=$(
    for binary in "${BINARIES[@]}"; do
        readelf -W --dyn-syms "$binary" | sed -n 's/.*GLIBC_\([0-9.]*\).*/\1/p'
    done | sort -Vu | tail -1
)
if [ -z "$floor" ]; then
    echo "Error: no GLIBC_ version found in the built binaries" >&2
    exit 1
fi
echo "$floor" > dist/glibc-floor

# Every soname the binaries link is mapped to the package providing it, and an
# unmapped one stops the build: a hand-written package list is correct until a
# crate links something new, and nothing reports the day it stops being.
soname_package() {
    case "$1" in
        libc.so.6|libm.so.6|libdl.so.2|libpthread.so.0|librt.so.1|ld-linux-*.so.*) echo "libc6 (>= $floor)" ;;
        libgcc_s.so.1) echo "libgcc-s1" ;;
        *) return 1 ;;
    esac
}

depends=()
for binary in "${BINARIES[@]}"; do
    while read -r soname; do
        if ! package=$(soname_package "$soname"); then
            echo "Error: $binary links $soname, which maps to no package here" >&2
            echo "Add it to soname_package in $0 once you know which package ships it." >&2
            exit 1
        fi
        depends+=("$package")
    done < <(readelf -W -d "$binary" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p')
done

printf '%s\n' "${depends[@]}" | sort -u | paste -sd, - | sed 's/,/, /g' > dist/deb-depends

ls -l dist
echo "glibc floor: $floor"
echo "depends: $(< dist/deb-depends)"
