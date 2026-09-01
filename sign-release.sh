#!/bin/bash
# Detach-sign a draft release's assets, upload the signatures, publish the release.
set -euo pipefail

cd "$(dirname "$0")"

DRY_RUN=0
TAG=""
while (( $# )); do
    case "$1" in
        -n|--dry-run) DRY_RUN=1 ;;
        -h|--help)
            echo "usage: ${0##*/} [-n|--dry-run] [TAG]"
            exit 0
            ;;
        -*) echo "unknown option: $1" >&2; exit 1 ;;
        *) TAG="$1" ;;
    esac
    shift
done

TAG="${TAG:-v$(grep -Po '^version = "\K[^"]+' Cargo.toml)}"

# apt.ticpu.net verifies every asset against this fingerprint and rejects any
# other key by name, so the expectation lives here rather than in a per-machine
# git config: a checkout that never got the repo-local setting inherits a global
# signingkey and would otherwise sign with it.
RELEASE_KEY=E5998E49DC9E1DCFDB9B46EC77EBA10790CFFCCD

# set -e would kill the script on git config's exit 1, before the message below
KEY=$(git config user.signingkey || true)
if [[ -z "$KEY" ]]; then
    echo "git config user.signingkey is unset: refusing to guess a signing key" >&2
    echo "  git config user.signingkey $RELEASE_KEY" >&2
    exit 1
fi
# Resolved through gpg, since a key id, an email or a subkey all name the key
# without matching its fingerprint as text.
RESOLVED=$(gpg --list-secret-keys --with-colons "$KEY" | awk -F: '$1 == "fpr" { print $10; exit }')
if [[ "$RESOLVED" != "$RELEASE_KEY" ]]; then
    echo "user.signingkey is $KEY, which resolves to ${RESOLVED:-no secret key}" >&2
    echo "the archive verifies against $RELEASE_KEY and rejects anything else:" >&2
    echo "  git config user.signingkey $RELEASE_KEY" >&2
    exit 1
fi

WORKDIR="scratch/release-sign-$TAG"
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"

gh release download "$TAG" --dir "$WORKDIR"

# Signatures from an earlier run of this script are replaced, never re-signed
rm -f "$WORKDIR"/*.asc

shopt -s nullglob
assets=("$WORKDIR"/*)
if (( ${#assets[@]} == 0 )); then
    echo "$TAG has no assets to sign" >&2
    exit 1
fi

for asset in "${assets[@]}"; do
    gpg --local-user "$KEY" --armor --detach-sign --yes --output "$asset.asc" "$asset"
    gpg --verify "$asset.asc" "$asset"
done

if (( DRY_RUN )); then
    echo "dry run: signed ${#assets[@]} assets of $TAG with $KEY, nothing uploaded, release left as-is"
    echo "signatures left in $WORKDIR for inspection:"
    printf '  %s\n' "$WORKDIR"/*.asc
    exit 0
fi

gh release upload "$TAG" "$WORKDIR"/*.asc --clobber

# CI leaves the release a draft so it goes public only once signed. Assets of a
# published release cannot be added or replaced when release immutability is on.
gh release edit "$TAG" --draft=false

rm -rf "$WORKDIR"
echo "Signed ${#assets[@]} assets of $TAG with $KEY"
echo "Remove a signature with: gh release delete-asset $TAG <name>.asc"
