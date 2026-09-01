---
description: Cut a release — version, tag, package, deploy. Invoking this IS the permission.
---

Cut a release of the repo in the working directory. Target version: $ARGUMENTS (ask if empty
and the next version is not obvious from what is already published).

Invoking this command is the explicit permission a release needs. Nothing else in a session
grants it: "commit the fix" never means release.

## 1. Preflight

- `git pull --rebase`, then `git status`. A dirty tree that is not the version bump gets
  committed or stashed first — never swept into the release commit.
- Tests and lints green by the repo's own gate: `make -j check`, or
  `cargo clippy --fix --allow-dirty --message-format=short && cargo fmt` plus one
  `cargo test --release` where there is no Makefile.
- Read the deploy path before building. For a Rust binary shipped as a .deb, one Containerfile
  must cross-compile every arch in a single pass into `dist/`. If it still builds one arch per
  image, say so now — that is a fix before tagging, not after.

## 2. Pick the version against what is already published

Never reuse a version that has left this machine. Anything already holding it never upgrades.

- apt repository: search the published suite for the package name. The deploy script names the
  host and the repo; read it rather than typing either from memory.
- crates.io: `cargo info <crate> --registry crates-io` — the `--registry` is not optional
  inside a workspace, or you read the local path dep's version instead.

If the version you were about to cut is already there, bump instead and say why. A local tag
and a local release commit for that version are not evidence it was unpublished — check the
registry, not the log.

## 3. Do not rewrite a published release out of history

A release commit whose artifacts were uploaded stays. Redoing it means a new version on top,
not a rebase. Rewriting is only for a release that never left the machine, and even then the
tag has to be restored if the rebase conflicts and you abort:

    git update-ref refs/tags/vX.Y.Z <tag-object-sha>   # sha from `Deleted tag (was …)`

That restores the annotated tag whole — message included — which re-tagging by hand does not.

## 4. Bump, commit, tag

- Edit `version` in `Cargo.toml`, then build once so `Cargo.lock` picks it up.
- Binaries gitignore the lock during development, so the release commit is the one that carries
  it: `git add Cargo.toml && git add -f Cargo.lock`.
- Commit subject `release: vX.Y.Z`, no body. The per-step commit rule does not apply to release
  artifacts.
- `git tag -a vX.Y.Z -m "<changelog>"`. The changelog lives in the **tag**, never the commit:
  features, fixes, API changes, written for someone who does not follow development. Not a
  commit list, not hashes, no diffstat. Re-read it before moving on — a garbled sentence is
  fixed with `git tag -f -a`, and only while the tag is unpushed.
- Library crate: `cargo publish --dry-run` before any real publish. Never skip it.

## 5. Push, before anything is uploaded

`git push --follow-tags origin <branch>` sends the commits and the tags together. Ask first,
naming how many of each — it is outward-facing and not quietly undone.

It goes here, ahead of the upload, because a package in a repo whose commit is not on origin
cannot be reproduced by anyone but the machine that built it, and the tag is what names the
source it was built from. If the push is refused, stop: an unpushed release does not get
deployed.

## 6. Package and deploy

For the .deb path (`packaging/`):

- Delete the previous version's `*.deb` from the build directory first. A deploy script uploads
  every `.deb` it finds there, so a stale one is re-uploaded silently.
- `make -C packaging deb`, then check the built binary is newer than the sources you just
  changed — a stale `dist/` packages the previous release under the new version number.
- Smoke-test the packaged binary for this arch before uploading, not the one in `target/`.
- Run the repo's deploy script, then verify with the same search from step 2.

For AUR: no `Co-Authored-By`; edit pkgver, `updpkgsums`, `makepkg --printsrcinfo > .SRCINFO`,
commit both, push.

## Report

Version, tag, what was built, where it was deployed, what was verified, and whether the push
happened. If a step was skipped, say which and why.
