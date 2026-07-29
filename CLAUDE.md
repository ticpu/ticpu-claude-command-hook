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
- `git_bypass` — two decisions, both per chain segment. **Denies** `--no-verify` (unless the
  commit message starts with `test`), `--no-gpg-sign`, and `commit.gpgsign=false/0` on any
  segment, plus a non-read-only `git -C` pointing at the current workdir ("drop the -C"), plus
  `git add -A`/`.`/`-u`/`*` (CLAUDE.md: stage explicit paths — a plain `Bash(git add:*)`
  allowlist entry does not stop those). Bypass flags count only where git reads options: whole
  tokens, outside quotes, before a heredoc marker — so a commit message may name a flag it
  isn't using. **Allows** a command whose every segment is a bare `cd`, a provably read-only
  git pipeline, or a `git add` naming at least one path; the standard permission allowlist
  can't express that, and it covers `git -C <anypath> status` as well as
  `cd <path> && git diff|add …`, which Claude Code otherwise prompts about however the
  allowlist reads (hooks from the target directory — neither a read-only subcommand nor
  `git add` runs one, and staging is undone by `git restore --staged`). The allow is
  whole-command, not per segment,
  because one allow decides the whole call: a stdout redirect or a consumer that is not
  display-only (`| sh`, `; rm -rf`) keeps the prompt. It runs last in `dispatch` so every
  objection gets first say and a `git grep` still reaches the fold. Read-only classification
  is fail-safe — a whitelist of always-safe subcommands plus explicit read-only modes for the
  mode-dependent ones (branch/tag/config/remote/reflog/symbolic-ref); any unrecognized flag or
  verb prompts.
- `broad_find` — denies `find` walks of `/`, `~`, `$HOME`, the bare home dir, or the GIT
  repo parent; a find scoped to one repo under GIT is allowed.
- `design_rationale` — on any edit to a `design-rationale.md`, injects the stop-for-review
  reminder.
- `grep_fold` — rewrites searches to pipe through the sibling `gf`, per chain segment, so a
  chained or `cd`-prefixed grep still folds. `gf` lands after the *last* search stage, since a
  later `grep`/`rg` filters lines and its pattern can match the prefix gf strips; everything
  past that point must only display (a path-consuming `xargs`/`awk` would get truncated
  paths). Refuses a segment whose flags change the output shape gf parses, or that redirects
  stdout — a stderr-only redirect (`2>&1`, `2>file`) still folds, gf passes the error lines
  through.
  When gf ends the pipeline it would swallow the search's exit status, so `PIPESTATUS` puts it
  back, brace-grouped so it stays attached to that segment.
- `search_stderr` — denies `2>/dev/null` on a search; `-s`/`--no-messages` is the scoped
  alternative.

`command grep` is the documented opt-out from both: `shell::WRAPPERS` deliberately omits
`command`, so it never classifies as a search.

`src/checks/shell.rs` is the only shell parser — one quote mask feeds chain splitting,
pipeline splitting, redirect detection and unquoting. A second ad-hoc matcher already caused
one bug (a `2>/dev/null` inside a search *pattern* read as a real redirect).

## Adding a check

Add a module under `src/checks/`, wire it into `dispatch`, and unit-test the pure decision
function. Keep IO (filesystem, env) thin and behind a testable core (see `glab_skill::decide`).

## Build / test

`make -j check` (clippy `-D warnings` + `cargo test`) then `make release`. The hook entries
point at the absolute `target/release/ticpu-claude-command-hook` path, so rebuild after
changing a check — and `gf` must stay beside it, which `cargo build` handles.

`tests/verdicts.rs` runs the real binary over a table of commands and asserts pass / deny /
rewritten-command; add a row there for any new shape. `./probe.sh` prints the same verdicts
for commands on stdin when you just want to try one.

License GPL-3.0-only. Commits run `gitleaks git --staged` via `core.hooksPath=githooks`.
