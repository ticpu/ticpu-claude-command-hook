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
- **forge write gate** — creating, editing or commenting on a PR, MR or issue through `gh`
  or `glab` is always prompted, in every permission mode. A `gh` create must take its body
  from a `pr-body*.md` / `issue-body*.md` file, re-checked as the command runs.
- **PR body check** — a Write or Edit to such a file is refused while it carries a
  generated-with line, an emoji, a reference-style link, hard-wrapped prose or, for a PR, no
  testing section.
- **comment cap** — an Edit or Write that adds or changes a run of more than two whole-line
  `//` or `#` comments is refused. Doc comments, the block opening a file and a block carrying
  `comment-cap-exempt: <reason>` are left alone. `ticpu-claude-command-hook comment-ignore
  <repo-name>` turns it off for one repo, recorded in
  `$XDG_CONFIG_HOME/ticpu-claude-command-hook/config.yaml`.
- **broad find guard** — blocks `find` walks of `/`, `~`, `$HOME`, the bare home directory,
  or the parent directory holding all your repos; a `find` scoped to one repo is allowed.
- **remote session guard** — denies `ssh`, `sshfs`, `psql`, `mysql`, `mariadb` or `mongosh`
  bundled with another command (`;`, `&&`, `||`, `&`, or a second line), or fed by one
  (`cat dump.sql | mysql`). They must be the whole call, optionally piped into a viewer
  (`| jq`); a bare `echo` alongside is allowed, and chaining inside the quoted remote command
  or SQL body is the far end's business and passes through.
- **design-rationale gate** — an edit to a `docs/design-rationale.md` waits on a whole read
  of the file in the session. The countable rules are decided in code (a `## Why …` heading,
  a section past the length bound, a CLAUDE.md reference) and deny with the offending text
  quoted. Every other edit prompts for your approval whatever your permission rules say,
  the prompt naming the section it lands in, so the review happens before the write.
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

```
./target/release/ticpu-claude-command-hook install
```

Writes its own absolute path into `~/.claude/settings.json` (`CLAUDE_CONFIG_DIR` honoured)
as a `PreToolUse` entry matching `Bash|Edit|Write` and a `PostToolUse` one matching
`Edit|Write` — the tools it dispatches on. Everything else in the file is left alone,
including hooks that run something else; an entry already running a binary of this name is
replaced rather than duplicated, so re-running it after moving the checkout re-points the
old one. A session started before the write picks it up after `/hooks` or a restart.

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

## Release

```
./release.sh vX.Y.Z -F changelog.md
```

Preflight, version bump, `release:` commit carrying the lockfile, annotated tag, push, then
it waits on the tag's `release.yml` run, signs the draft the run created, and reads back the
published `.deb`: version, no `Depends`, no `DT_NEEDED`, and `rules` output matching
`docs/allowed-commands.md`. The artifact is checked rather than the tree it was built from,
a stale `dist/` being how the previous release gets packaged under the new number.

The last step publishes those signed `.deb` files to [apt.ticpu.net](https://apt.ticpu.net),
and it is not optional-by-default: a release page nobody's `apt-get` reads is half a release.
`--no-apt` skips it for a release that is deliberately not going to the archive.

Each step asks whether it is already done, so an interrupted release resumes by re-running
the same command. A tag already on origin is never re-pointed and an existing release
refuses the run rather than replacing its assets: a published version is cut again by
bumping, not by rewriting.

## License

GPL-3.0-only.
