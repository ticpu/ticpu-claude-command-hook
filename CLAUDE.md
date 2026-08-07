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
  `$XDG_RUNTIME_DIR/claude-hooks/` lets later calls through. Any pipeline stage of any segment
  counts, so a `cd`, a wrapper or an absolute path does not skip the gate.
- `git_bypass` — two decisions, both per chain segment. **Denies** `--no-verify` (unless the
  commit message starts with `test`) in every spelling git accepts it — the long flag, a
  wholly-quoted `"--no-verify"`, and the `-n` that means it on `commit` alone — plus
  `--no-gpg-sign` and `commit.gpgsign=` set to any of git's four off values, case-insensitively,
  plus a non-read-only `git -C` pointing at the current workdir ("drop the -C"), plus
  `git add -A`/`.`/`-u`/`*` quoted or not (CLAUDE.md: stage explicit paths — a plain
  `Bash(git add:*)` allowlist entry does not stop those), plus a `cd` before a `git commit` (a commit is
  repo-wide, so the `cd` buys nothing and runs the *target* repo's hooks — the one case Claude
  Code's warning is literally about). That last one is the only check that does not go through
  `shell`: the shape worth catching is `-m "$(cat <<EOF …)"`, which the parser refuses on
  principle, so it walks tokens over the text before the heredoc marker with balanced quotes
  stripped — a message describing the rule does not trip it.
  Bypass flags count only where git reads options: whole
  tokens, outside quotes, before a heredoc marker — so a commit message may name a flag it
  isn't using. `git` is recognized by `shell::program`, so a path, a wrapper or a brace group
  carries the same denies a bare `git` does. **Allows** a command whose every segment is a bare
  `cd`, a lone `echo`, a
  provably read-only git pipeline, or a `git add` naming at least one path that exists as a
  file — a directory, glob or variable stages whatever is under it, which is the sweep the
  blanket forms are denied for; the allowlist
  can't express that, and it covers `git -C <anypath> status` as well as
  `cd <path> && git diff|add …`, which Claude Code otherwise prompts about however the
  allowlist reads (hooks from the target directory — neither a read-only subcommand nor
  `git add` runs one, and staging is undone by `git restore --staged`). The allow is
  whole-command, not per segment,
  because one allow decides the whole call: any redirect at all — `2>file` truncates what it
  names even though it is not a stdout redirect — or a consumer that can write or
  run something (`| sh`, `| sed -i`, `; rm -rf`), keeps the prompt. A consumer here only has to
  add no side effect of its own, which is weaker than grep_fold's display-only test — that one
  also has to survive gf's folding — so `wc` and a line-selecting `sed` qualify here and not
  there. It runs last in `dispatch` so every
  objection gets first say and a `git grep` still reaches the fold. Read-only classification
  is fail-safe — a whitelist of always-safe subcommands plus explicit read-only modes for the
  mode-dependent ones (branch/tag/config/remote/reflog/symbolic-ref); any unrecognized flag or
  verb prompts, as does an option that writes a file or runs a program (`--output=`, `-O`) and
  any `-c`, which can point config at a program under a read-only verb.
- `lone_cd` — denies a command whose every segment is just a `cd`. Each Bash call gets a fresh
  shell, so nothing observes the change; the shape only shows up as a retry after a chained `cd`
  was refused, which splitting cannot fix. A redirect or a pipe means the segment leaves
  something behind, so it is not the no-op this denies.
- `broad_find` — denies `find` walks of `/`, `~`, `$HOME`, the bare home dir, or the GIT
  repo parent; a find scoped to one repo under GIT is allowed.
- `remote_session` — denies an `ssh`/`sshfs`/`psql`/`mysql`/`mariadb`/`mongosh` bundled with
  anything else: no `;`, `&&`, `||`, `&`, and no unquoted newline. A lone `echo` is not
  company (`… ; echo "rc=$?"` is routine), and neither is a wrapper — `shell::command_word`
  reads through `sudo -u postgres psql` and `timeout 45 ssh`. The client must also lead its
  pipeline, since a producer feeding it rides along on its approval; a consumer after it
  (`| jq`) is fine, and chaining inside the quoted remote command or SQL body is the far
  end's. A heredoc is judged on the text before the marker — the body is data, so the usual
  `psql <<EOF` shape passes, at the cost of not seeing a chain past the terminator.
- `design_rationale` — on any edit to a `design-rationale.md`, injects the stop-for-review
  reminder.
- `grep_fold` — rewrites searches to pipe through the sibling `gf`, per chain segment, so a
  chained or `cd`-prefixed grep still folds. `gf` lands after the *last* search stage, since a
  later `grep`/`rg` filters lines and its pattern can match the prefix gf strips; everything
  past that point must only display (a path-consuming `xargs`/`awk` would get truncated
  paths). Refuses a segment whose flags change the output shape gf parses, that runs a program
  of its own (`--pre`, `git grep -O`), or that redirects
  stdout — a stderr-only redirect (`2>&1`, `2>file`) still folds, gf passes the error lines
  through.
  When gf ends the pipeline it would swallow the search's exit status, so `PIPESTATUS` puts it
  back, brace-grouped so it stays attached to that segment.
  A rewrite is only honoured next to an `allow`, and that allow covers the *whole* call — so
  the fold is emitted only when every segment is one it can vouch for (a folded search, a bare
  `cd`, a read-only utility). A chain carrying anything else keeps its prompt and forfeits the
  fold, rather than having the fold grant it permission.
- `search_stderr` — denies `2>/dev/null` on a search; `-s`/`--no-messages` is the scoped
  alternative.
- `search_flags` — two denies for flags a grep habit reads wrong. `rg -r` in any form is
  `--replace`, so `rg -rn PAT dir` prints every hit rewritten to `n` and the damage reads as
  ordinary output; `--replace=` is the unambiguous spelling. And a search filtering another
  search's output may not carry `-n`/`-b`/`-H`/`--vimgrep`: that prefix counts the piped stream,
  so the numbers belong to no file. Flag scanning is cluster-aware per tool — a short flag that
  takes a value swallows the rest of its cluster (`rg -trust` is `--type rust`, not `-r ust`),
  and the next word too when nothing is glued on.

`command grep` is the documented opt-out from both: `shell::WRAPPERS` deliberately omits
`command`, so it never classifies as a search.

`src/checks/shell.rs` is the only shell parser — one mask feeds chain splitting, pipeline
splitting, redirect detection and unquoting. It marks the bytes outside quotes *and* outside
command substitutions, so `grep -rn foo $(pwd) 2>/dev/null` still reads as a search with a
silenced stderr; only a heredoc or an unbalanced quote makes a command unanalyzable. A newline
separates commands like `;` does. A second ad-hoc matcher already caused one bug (a
`2>/dev/null` inside a search *pattern* read as a real redirect), and every check that grew its
own notion of "is this git / glab / a search" grew an evasion with it — ask `shell::program`.

## Adding a check

Add a module under `src/checks/`, wire it into `dispatch`, and unit-test the pure decision
function. Keep IO (filesystem, env) thin and behind a testable core (see `glab_skill::decide`).

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

`tests/verdicts.rs` runs the real binary over a table of commands and asserts pass / deny /
rewritten-command; add a row there for any new shape. `./probe.sh` prints the same verdicts
for commands on stdin when you just want to try one.

License GPL-3.0-only. Commits run `gitleaks git --staged` via `core.hooksPath=githooks`.
