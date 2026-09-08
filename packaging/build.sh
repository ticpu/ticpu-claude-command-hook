#!/bin/bash
# Cross-compiles every release target in one container build and writes the
# binaries to packaging/dist/. The binaries are static musl, so there is no
# glibc floor to compute and no Depends for the control file to carry; the
# container build asserts the absence of DT_NEEDED instead.

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

ls -l dist
