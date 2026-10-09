# ticpu-claude-command-hook

Single binary that backs every Claude Code hook entry in `~/.claude/settings.json`.
Replaces a pile of per-check shell scripts: one Rust program, reached straight from
`target/release/`, with `cargo test` covering each check.

## How it works

`main` reads the hook JSON from stdin (`serde_json`), and `checks::dispatch` routes on
`hook_event_name` + `tool_name`. Each check is a module under `src/checks/` returning
`Option<HookOutput>`: `Some` objects to the action, `None` allows. First objection wins.

Output is the documented hook JSON on stdout (`src/output.rs`):

- PreToolUse block → `hookSpecificOutput.permissionDecision = "deny"` with a reason.
- PreToolUse rewrite → `updatedInput` replacing the whole tool input. Claude Code only
  honours it next to an `"allow"` decision, so a rewrite also skips the permission prompt;
  `HookInput::tool_input` stays a raw `Value` so a rewrite hands back the fields this
  binary does not model (`description`, `timeout`, …) untouched.
- PostToolUse advisory → `systemMessage` + `hookSpecificOutput.additionalContext`.

Exit code is always 0 except on an internal error (bad stdin, serialize failure), which
exits 1 with context on stderr. Fail-open is deliberate: a bug here must never block the
user's tools. Checks never silence their own IO errors — they log and allow.

What each check does, what it exempts and why is in the notes at the top of its own file.
`checks/mod.rs` states the order they run in and the gate every allow sits behind;
`checks/shell.rs` is the only shell parser and `checks/location.rs` the only answer to where
a segment runs — ask them rather than matching text in a check.

The one-shot waivers and the standing switch are listed in `docs/waivers.md` with the
command that creates each. It is written for the user running one by hand, so a new marker
goes in it as well as in its check.

## Adding a check

Add a module under `src/checks/`, wire it into `dispatch`, and unit-test the pure decision
function. Keep IO (filesystem, env) thin and behind a testable core (see `glab_skill::decide`).
Its notes go at the top of its file: what it denies or allows, each exemption with its
reason. A deny is the only place its rule is stated, so it says the rule and what to do
instead without pointing at any CLAUDE.md.

A message the check emits stays a literal beside the branch that emits it until it passes
a kilobyte; past that it moves to a `*.txt` read with `include_str!` (`glab-traps.txt`) and
is rewritten by `/compress-messages`, which names what it must not touch. A deny stays in
the transcript for the rest of the session, so one that fires more than once a session is
paid for every time.

A check on `Edit`/`Write` that judges a whole file reads it through `checks/edited.rs`.

A check that can *allow* also states its shape in `src/rules.rs`: a caller cannot infer an
allow from a refusal it never sees, and one it cannot predict it does not use.

## Working here

`git pull --rebase` before touching anything. This repo is edited from several machines and
from Claude sessions that outlive each other, so a stale checkout is the normal case, not the
exception — and the binary it builds is live in `~/.claude/settings.json`, so diverging here
means the running hook stops matching the source.

Leave nothing uncommitted at the end of a session: every finished step gets its own commit
before the next one starts, and the last one gets pushed. Formatting churn counts — commit it
on its own (`style:`) rather than folding it into a behaviour change.

## Build / test

`make -j check` (clippy `-D warnings` + `cargo test`) then `make release`. The hook entries
point at the absolute `target/release/ticpu-claude-command-hook` path, so rebuild after
changing a check — and `gf` must stay beside it, which `cargo build` handles.

`ticpu-claude-command-hook install` writes those entries itself, matching by binary name so
a re-run after moving the checkout re-points the old entry instead of adding a second one.
It is also the only place the matchers are stated, so a new event or tool in `dispatch`
needs `ENTRIES` in `src/install.rs` widened to match. It prints the `@` line importing
`docs/allowed-commands.md`, which `make release` regenerates from `src/rules.rs` — a
`CLAUDE.md` importing that file must never lag the binary deciding the allows. That text is
imported into every session and priced per token, so it is written flat: one line per shape,
no headings, bullets, blank lines or backticks, and no sentence that explains rather than
states. Command words, flags and paths survive verbatim; nothing else has to.

`make install` and `make uninstall` are those two verbs over the built binary. `uninstall`
matches the same way install does — by binary name, under every event rather than the ones
`ENTRIES` lists today — so an entry left by an older build or a checkout that has since moved
goes with the rest, and a group or event holding nothing else goes with it. It touches no
other hook and no other setting, and it leaves the binary and `docs/allowed-commands.md`
alone: it takes the hook out of Claude Code, it does not undo the build.

`make archpkg-install` builds `packaging/PKGBUILD` natively and hands the package to
pacman — the Arch counterpart of the cross-compiled `.deb`, same layout (`/usr/bin`,
`gf` under `/usr/libexec/<name>/`). `make archpkg` stops at the built package. It builds
the checkout it sits in, not a tarball, so it is a local-install path and not a
distributable PKGBUILD. After installing, `ticpu-claude-command-hook install` re-points
`settings.json` at `/usr/bin` — the entries otherwise still name this checkout's
`target/release/`.

`tests/verdicts.rs` runs the real binary over a table of commands and asserts pass / deny /
rewritten-command; add a row there for any new shape. `./probe.sh` prints the same verdicts
for commands on stdin when you just want to try one.

License GPL-3.0-only. Commits run `gitleaks git --staged` from `githooks/pre-commit`.
