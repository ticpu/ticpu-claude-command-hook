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

## Build

```
cargo build --release
```

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
