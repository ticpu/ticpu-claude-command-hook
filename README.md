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
- **git bypass guard** — blocks `--no-verify` (except commit messages starting with
  `test`), `--no-gpg-sign`, and `-c commit.gpgsign=false`.
- **broad find guard** — blocks `find` walks of `/`, `~`, `$HOME`, the bare home directory,
  or the parent directory holding all your repos; a `find` scoped to one repo is allowed.
- **design-rationale review gate** — when an edit touches a `design-rationale.md`, injects a
  reminder to stop and present the diff for review.
- **grep fold** — rewrites a plain `grep`/`rg`/`git grep` command to pipe through `gf`, so
  repeated file paths collapse instead of eating the model's context. Only plain pipelines
  whose later stages just display (`head`, `tail`, `less`, `cat`, `nl`) are rewritten;
  anything that parses the path back out is left alone.

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
cargo test
```

Each check lives in `src/checks/` and returns `Option<HookOutput>`. Add a module, wire it
into `checks::dispatch`, and unit-test the decision function.

## License

GPL-3.0-only.
