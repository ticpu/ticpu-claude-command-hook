# ticpu-claude-command-hook

A single Rust binary that backs multiple [Claude Code](https://claude.com/claude-code)
hook entries — instead of one shell script per check, one tested program dispatches them
all from `target/release/`.

It reads the hook JSON on stdin, decides based on the event and tool, and emits the
documented hook-output JSON. Exit status is always 0 except on an internal error (exit 1,
fail-open) so a bug in the hook never blocks your tools.

## Checks

- **glab skill gate** — denies the first `glab` command per session so the `glab` skill
  gets loaded; later calls pass (tracked by a marker in `$XDG_RUNTIME_DIR/claude-hooks/`).
  A leading `cd`, a wrapper or an absolute path does not skip it.
- **git bypass guard** — blocks `--no-verify` (except commit messages starting with
  `test`), `--no-gpg-sign`, and `-c commit.gpgsign=false`, in every spelling git accepts:
  quoted, short (`commit -n`), and the other off values. `git` is recognized behind a path,
  a wrapper or a brace group.
- **broad find guard** — blocks `find` walks of `/`, `~`, `$HOME`, the bare home directory,
  or the parent directory holding all your repos; a `find` scoped to one repo is allowed.
- **remote session guard** — denies `ssh`, `sshfs`, `psql`, `mysql`, `mariadb` or `mongosh`
  bundled with another command (`;`, `&&`, `||`, `&`, or a second line), or fed by one
  (`cat dump.sql | mysql`). They must be the whole call, optionally piped into a viewer
  (`| jq`); a bare `echo` alongside is allowed, and chaining inside the quoted remote command
  or SQL body is the far end's business and passes through.
- **design-rationale review gate** — when an edit touches a `design-rationale.md`, injects a
  reminder to stop and present the diff for review.
- **grep fold** — rewrites `grep`/`rg`/`git grep` commands to pipe through `gf`, so repeated
  file paths collapse instead of eating the model's context. Chains are handled per segment
  (`cd /x && grep …` folds the grep and leaves the `cd`), and a segment that cannot be
  folded costs only itself. `gf` is spliced in after the last search stage — `rg … | rg -v
  'some.xml'` filters on whole paths, so folding before it would change what matches. Left
  alone: stages past that point that do anything but display (`head`, `tail`, `less`, `cat`,
  `nl` are fine; `xargs`, `awk`, `sort`, `wc` are not), redirects, `-q`/`-Z`/`-z`, search
  options that run a program (`--pre`, `git grep -O`), and anything with a heredoc. Because
  Claude Code only honours a rewrite next to an `allow` — which covers the whole call — a chain
  is folded only when every segment is one the fold can vouch for; `grep … ; rm -rf …` keeps
  its prompt instead. Writing `command grep` opts out entirely.
- **search stderr guard** — denies `2>/dev/null` on a search: it hides wrong paths and
  unreadable dirs, and `-s`/`--no-messages` suppresses just the file noise instead.

## gf

The second binary in this crate. It reads grep-style output and prints a file path once
per run of consecutive lines from the same file, dropping configured directory prefixes:

```
$ grep -rn notify_command -A 2 ~/GIT/ng911/rust/test-data/ | gf
base: /home/jerome.poulin/GIT/
ng911/rust/test-data/deploy-configs/localhost/noans-worker-lab/config.yaml:44:  notify_command:
-45-    endpoint:
-46-      loopback:
```

Prefixes come from `--strip PREFIX` (repeatable), the `:`-separated `GF_STRIP`, and `$PWD`.
Each one is announced once with a `base:` line, so the full paths stay recoverable;
`--no-base` drops that. With arguments and no `--stdin`, `gf` runs `grep` (or `--cmd PROG`
/ `GF_CMD`) itself and exits with its status. `gf --help` covers the rest.

A path is recognized as the shortest prefix that both is followed by `SEP digits SEP`
(`:44:`, `-45-`) and exists on disk — results are cached, so repeats cost no syscalls.
Everything else is passed through byte-for-byte, ANSI escapes included, so `--color=always`
still works. Paths containing `:` are not detected, paths containing spaces only on
line-numbered output, and `-Z/--null` output is unsupported.

## Build

```
cargo build --release
```

Builds both binaries; the hook finds `gf` as its own sibling in `target/release/`, so a
`gf` elsewhere on `PATH` is never used for the rewrite.

## Wire into Claude Code

Point your hook entries at the built binary in `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "/path/to/ticpu-claude-command-hook/target/release/ticpu-claude-command-hook" }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [
          { "type": "command", "command": "/path/to/ticpu-claude-command-hook/target/release/ticpu-claude-command-hook" }
        ]
      }
    ]
  }
}
```

The binary self-dispatches on the hook event and tool name, so the same path serves every
entry.

## Develop

```
make -j check          # clippy -D warnings + cargo test
echo 'grep -rn x src' | ./probe.sh    # what would the hook do with this command?
```

Each check lives in `src/checks/` and returns `Option<HookOutput>`. Add a module, wire it
into `checks::dispatch`, and unit-test the decision function. `src/checks/shell.rs` holds the
one quote-aware splitter every command-shape question goes through — don't grow a second one.
`tests/verdicts.rs` is the asserted verdict table, run against the real binary; `probe.sh`
answers the same question for one-off commands.

## License

GPL-3.0-only.
