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

## Checks

- `glab_skill` — first `glab` per session is denied to force `Skill("glab")`; a marker in
  `$XDG_RUNTIME_DIR/claude-hooks/` lets later calls through.
- `git_bypass` — denies `--no-verify` (unless the commit message starts with `test`),
  `--no-gpg-sign`, and `commit.gpgsign=false/0`. Also handles `git -C <path>`: a
  provably read-only subcommand is auto-*allowed* (the standard permission allowlist
  can't express `git -C <anypath> status`), while a non-read-only `-C` pointing at the
  current workdir is denied with a "drop the -C" reason; anything else falls through to
  the normal prompt. Read-only classification is fail-safe — a whitelist of always-safe
  subcommands plus explicit read-only modes for the mode-dependent ones (branch/tag/
  config/remote/reflog/symbolic-ref); any unrecognized flag or verb prompts.
- `broad_find` — denies `find` walks of `/`, `~`, `$HOME`, the bare home dir, or the GIT
  repo parent; a find scoped to one repo under GIT is allowed.
- `design_rationale` — on any edit to a `design-rationale.md`, injects the stop-for-review
  reminder.

## Adding a check

Add a module under `src/checks/`, wire it into `dispatch`, and unit-test the pure decision
function. Keep IO (filesystem, env) thin and behind a testable core (see `glab_skill::decide`).

## Build / test

`cargo test` then `cargo build --release`. The hook entries point at the absolute
`target/release/ticpu-claude-command-hook` path, so rebuild after changing a check.

License GPL-3.0-only. Commits run `gitleaks git --staged` via `core.hooksPath=githooks`.
