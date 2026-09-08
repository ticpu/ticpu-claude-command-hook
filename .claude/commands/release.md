---
description: Cut a release — version, tag, package, deploy. Invoking this IS the permission.
---

Cut a release of this repo. Target version: $ARGUMENTS (ask if empty and the next version is
not obvious from what is already published).

Invoking this command is the explicit permission a release needs. Nothing else in a session
grants it: "commit the fix" never means release.

## The whole release is one script

```
./release.sh vX.Y.Z -F - <<'EOF'
<changelog>
EOF
```

It preflights (master, clean tree, `git pull --rebase`, `make -j check`), bumps `Cargo.toml`,
commits `release: vX.Y.Z` with the force-added `Cargo.lock`, tags annotated, pushes, waits on
the tag's `release.yml` run, signs and publishes the draft, verifies the published `.deb`, and
publishes it to apt.ticpu.net. Read `release.sh` before working around any of it.

Every step asks whether it is already done, so an interrupted release is resumed by re-running
the same command. Do not unpick one by hand: a tag already on origin is never re-pointed, and a
published version is superseded by a bump, never by rewriting.

**The apt publish is not a question.** Every release goes to the archive, without asking and
without waiting for a follow-up instruction — a release page nobody's `apt-get` reads is half a
release. `--no-apt` exists for a release deliberately kept off the archive, which is not the
default and not something to choose on the model's own initiative.

The one thing the script does not decide is the version. Never reuse one that has left this
machine: check `gh release list` and the archive, not the local log — a local tag and a local
`release:` commit are not evidence a version was never published.

## Changelog

Goes in the tag message, which the script feeds to the GitHub release body. Written for someone
not following development: features, fixes, behaviour changes, in user-visible terms. No commit
lists, no hashes, no diffstat, no co-author lines. One line per paragraph — the forge reflows it,
and a hard-wrapped body breaks awkwardly on a phone.

## Report

Version, tag, what CI built, what was verified, whether the archive got it, and any step that
was skipped with the reason.
