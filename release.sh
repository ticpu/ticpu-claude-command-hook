#!/bin/bash
# Cut a release: preflight, bump, tag, push, wait for CI, sign, verify.
#
# Usage: ./release.sh vX.Y.Z [-F <changelog-file>|-] [--no-push]
#          -F        the tag's changelog; "-" reads stdin, absent opens $EDITOR
#          --no-push stop after the tag, so it can be read before it leaves
#
# Every step asks whether it is already done, so an interrupted release resumes
# by re-running the same command instead of being unpicked by hand. Nothing here
# rewrites a published version: a tag already on origin is never re-pointed, and
# a release that exists refuses the run rather than replacing its assets.

set -euo pipefail

cd "$(dirname "$0")"

die() { echo "$*" >&2; exit 1; }

TAG=""
CHANGELOG_FILE=""
PUSH=1
while (($#)); do
	case "$1" in
		-F | --file)
			CHANGELOG_FILE="$2"
			shift
			;;
		--no-push) PUSH=0 ;;
		-h | --help)
			sed -n '2,6p' "$0"
			exit 0
			;;
		-*) die "unknown option: $1" ;;
		*) TAG="$1" ;;
	esac
	shift
done

[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
	die "usage: ${0##*/} vX.Y.Z [-F <changelog-file>|-] [--no-push]"
VERSION="${TAG#v}"

# Read before anything is changed: a "-" that turns out to be an empty stdin
# should stop the release, not the tagging step halfway through it.
CHANGELOG=""
if [ -n "$CHANGELOG_FILE" ]; then
	CHANGELOG="$(cat -- "$CHANGELOG_FILE")"
	[ -n "$CHANGELOG" ] || die "$CHANGELOG_FILE is empty: the tag carries the changelog"
fi

# gh reports a missing release as an error, which is the answer here rather than
# a failure — but only that one, so anything else is passed through.
release_state() {
	local out
	if out="$(gh release view "$TAG" --json isDraft --jq 'if .isDraft then "draft" else "published" end' 2>&1)"; then
		printf '%s' "$out"
	elif grep -qi 'release not found' <<<"$out"; then
		printf 'none'
	else
		echo "$out" >&2
		die "gh release view $TAG failed"
	fi
}

have_local_tag() { git rev-parse -q --verify "refs/tags/$TAG" > /dev/null; }
have_remote_tag() { [ -n "$(git ls-remote --tags origin "refs/tags/$TAG")" ]; }

state="$(release_state)"

if ! have_local_tag; then
	[ "$state" = none ] || die "$TAG is already released ($state): bump instead, a published version never comes back"
	have_remote_tag && die "$TAG is already on origin: bump instead"

	branch="$(git symbolic-ref --short HEAD)"
	[ "$branch" = master ] || die "on branch $branch: releases are cut from master"

	git pull --rebase
	[ -z "$(git status --porcelain)" ] || die "working tree is dirty: commit or stash before releasing"

	make -j check

	# `make release` regenerates docs/allowed-commands.md, and a diff there means
	# the committed doc lagged the binary — a change of its own, not a release.
	sed -i -e "0,/^version = /s/^version = .*/version = \"$VERSION\"/" Cargo.toml
	make release
	unexpected="$(git status --porcelain | awk '$2 != "Cargo.toml" && $2 != "Cargo.lock"')"
	[ -z "$unexpected" ] ||
		die "the bump touched more than Cargo.toml and Cargo.lock:"$'\n'"$unexpected"

	# The lockfile is gitignored during development; the release commit is the one
	# that carries it, and CI refuses a tag without it.
	git add Cargo.toml
	git add -f Cargo.lock
	git commit -m "release: $TAG"

	if [ -n "$CHANGELOG" ]; then
		git tag -a "$TAG" -F - <<< "$TAG"$'\n\n'"$CHANGELOG"
	else
		git tag -a "$TAG"
	fi
fi

git rev-parse -q --verify "refs/tags/$TAG" > /dev/null ||
	die "$TAG was not created: nothing to push"

if [ "$PUSH" = 0 ]; then
	echo "stopping before the push: $TAG is tagged locally"
	echo "resume with: ${0##*/} $TAG"
	exit 0
fi

if ! have_remote_tag; then
	git push --follow-tags origin master
fi

./watch-ci.sh "$TAG" release.yml

state="$(release_state)"
case "$state" in
	none) die "CI created no release for $TAG" ;;
	draft) ./sign-release.sh "$TAG" ;;
	published) echo "$TAG is already published: leaving its assets alone" ;;
esac

# What is verified is the published .deb, not the tree it was built from: a
# stale dist/ or a mismatched tag would package the previous release under this
# version and nothing before this point reads the artifact.
WORKDIR="scratch/release-verify-$TAG"
rm -rf "$WORKDIR"
mkdir -p "$WORKDIR"
gh release download "$TAG" --dir "$WORKDIR" --pattern "*_${VERSION}_amd64.deb"
deb="$WORKDIR/ticpu-claude-command-hook_${VERSION}_amd64.deb"

control="$(dpkg-deb -f "$deb")"
[ "$(grep -Po '^Version: \K.*' <<< "$control")" = "$VERSION" ] ||
	die "$deb declares a version other than $VERSION:"$'\n'"$control"
# musl builds link nothing, so a Depends line means the build fell back to glibc
grep -q '^Depends:' <<< "$control" &&
	die "$deb declares Depends: the binaries are no longer static"

dpkg-deb -x "$deb" "$WORKDIR/root"
binary="$WORKDIR/root/usr/bin/ticpu-claude-command-hook"
readelf -d "$binary" | grep NEEDED &&
	die "the packaged binary links a shared library"

# The doc is generated from the binary's own rules, so a mismatch means the
# release shipped a doc describing allows it does not make.
diff -u docs/allowed-commands.md <("$binary" rules) ||
	die "the packaged binary's rules differ from docs/allowed-commands.md"

rm -rf "$WORKDIR"
echo "$TAG published and verified: $(gh release view "$TAG" --json url --jq .url)"
