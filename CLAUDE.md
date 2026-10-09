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

- `secret_paths` — a path that names a credential is denied wherever the command would print
  what it reads, since the output goes to the transcript and a value that reaches one is spent.
  It runs *first* in `dispatch`: `grep_fold` and `git_bypass::allow_safe` both emit an allow, and
  a `grep` of a secrets file must never reach one. A path lying inside a `$( )` is the exception
  — `URI=$(yq -r … secrets.yaml)` and `mongosh "$(…)"` keep their normal prompt — because a
  refusal that also blocks the credential's legitimate use leaves the model nothing to retry
  with; the substitutions are lifted out and only the outer text decides, so an assignment or a
  program passes and a printer among the outer tokens does not. What that program then does with
  the value is beyond this — which is also why a path given as the value of a key flag (`ssh -i`,
  a client's `--sslkey`) is not a print, nor is one named by a command that opens nothing (a mode
  change, a rename, a `stat`, a `test`, a `git log` carrying no patch flag). A search's pattern is exempt
  wherever it sits — `search_flags::pattern_words` tells it from the paths, since a pattern is the
  one argument a command names and does not open, and `rg 'password|secret' src/` was refused for
  quoting the word it looks for. A `jq`-family filter is the same argument under another name —
  `jq -r '.[]?|.key'` was refused for a field spelled like a key extension — so the one positional
  before the paths is exempt too, located by walking the flags rather than by how it reads. An
  long option it does not know is read as a switch, so one taking a path would exempt that path
  instead, and `-f` moves the filter into a file, which leaves every positional a path. A quoted,
  terminated heredoc is read as its head alone, the body being literal text nothing opens — the
  commit message describing this refusal was itself refused for quoting the filter. A printer's own words go the same way — it opens nothing, so a
  credential name among them is text on its way to the screen and only a substitution inside it
  can have read a file. A name matches on the basename (a word saying what it holds, a known
  credential dotfile, an `id_` key without `.pub`, a key or keystore extension) or on a directory
  component whose contents are credentials whatever the file inside is called; a source or prose
  extension exempts the *wording* rule alone, so `read_secret_management.rs` reads normally, and
  a directory keeps its meaning. A template word anywhere in the name (`secrets.yaml.sample`) and
  a ciphertext extension (`.eyaml`, `.gpg`) exempt the name rules whole rather than the wording
  alone, the values being removed or encrypted before either file is committed — the directory
  rule still standing over both, so a `.gpg` under `.password-store` is refused. A name that
  resolves has to exist before it counts — a bare word naming no file here is a word, not a path —
  while a glob or a variable, having nothing to stat, is judged on its wording. A resolved name
  git already tracks gives the name rules way: a committed credential is spent the day it landed,
  and what this is watching for is the ones living on the box outside any repo. The directory rule
  does not give way with them — a `.ssh` inside a checkout still holds keys, tracked or not. `Read` and `Grep` are matched on
  the path they name by the same rules; `Edit`/`Write` are not, a write printing nothing. A
  command `shell` cannot split is judged whole rather than waved through: this is the one check
  with no allow to withhold, so being wrong costs a prompt. A name that reads like a credential
  and is not one is answered by a waiver — `marker.rs`, the same shape the judge's bypass uses,
  under a name carrying none of the words above so that creating it is not itself refused; it is
  named in the deny, prompted on creation and spent by the next refusal. Not caught, deliberately: a
  recursive search rooted at a directory that merely contains one, a `Grep` `glob` (a repo-wide
  `*secret*` is ordinary), a value captured and later echoed, a path printed into a consumer that
  then reads it, and a quoted argument holding a space, which splits into words the pattern and
  path rules then judge one by one.
- `glab_skill` — first `glab` per session is denied; a marker in
  `$XDG_RUNTIME_DIR/claude-hooks/` lets later calls through. Any pipeline stage of any segment
  counts, so a `cd`, a wrapper or an absolute path does not skip the gate. The denial *carries*
  the guidance rather than pointing at `Skill("glab")`: a hardcoded list of traps, then
  `~/.claude/skills/glab/SKILL.md` (`CLAUDE_CONFIG_DIR` honoured) with its frontmatter dropped.
  A deny reason reaches the model, so this spends the same round trip the gate already cost and
  the retry is the corrected command instead of a detour through the skill tool. glab ships that
  file itself (`glab skills install --path ~/.claude/skills`), which is why the check reads it
  instead of embedding it — but a `--path` install is invisible to `glab skills update`, so it is
  refreshed by re-running install. The traps are the part the shipped skill omits: which `api`
  calls now have subcommands, and which verbs mean the opposite of what they read like
  (`repo archive` downloads). Refresh them when glab grows a subcommand for something the list
  still sends to `api`. A missing skill file degrades to the traps plus an install hint, since
  the traps are the half that cannot be recovered by loading anything.
- `sudo_journal` — denies a `journalctl` run under `sudo`, here or as the command an `ssh` hands
  the far end. Reading the journal comes from systemd-journal group membership, so the elevation
  changes nothing about what prints and asks for a password this shell cannot answer; the deny
  says so, and the retry is the same command without it. `systemctl` is not covered — its writes
  do need root, and a `sudo` in front of one is the shape it is for. The `sudo` is found by
  `leading_word` and the program under it by `shell::program`, so a wrapper between them
  (`sudo timeout 30 journalctl`) still counts. Over ssh the body is taken from the first `sudo`
  onwards rather than from the destination, which is what saves an option table this deny does
  not need — a miss costs a prompt, `systemd_read` allowing nothing that carries a wrapper at
  either end. Judged in front of a heredoc, so a commit message naming the refusal is prose.
- `broad_walk` — denies `find` walks of `/`, `~`, `$HOME`, the bare home dir, or the GIT
  repo parent; a find scoped to one repo under GIT is allowed. A trailing glob is judged
  on its parent, `~/GIT/*` being that same walk under another spelling. An `ls`/`tree` of
  the GIT parent or the home dir goes the same way, for a different reason: those two hold
  hundreds of entries and a repo path is built from its name, so the listing is browsing to
  guess rather than reading an answer. `/` is off that half — it prints two dozen names and
  answers a real question — and so is `ls -d`, which names the directory instead of
  listing it, while `tree -d` still walks. A lister is a neutral segment everywhere else,
  so this has to deny ahead of the allow that would otherwise carry it.
- `idle_burn` — denies a command whose every segment only passes time or hands back an exit
  status (`sleep`, `usleep`, `true`, `false`, `:`), an `echo` labelling the wait not
  counting as company. Nothing here is waiting to be polled: background work re-invokes the
  model when it finishes, a foreground `sleep` is refused by the harness anyway, and a
  condition is what the Monitor tool is for — so a bare `sleep 60` run in the background buys
  a turn and its own tool result and nothing else, which is why the deny says to end the turn
  instead. `wait` is off the list: it blocks on jobs the shell started, which is a real one.
  One idle segment is enough to deny, but
  only where the whole chain idles: `sleep 2 && curl …` waits for something, and a poll loop
  names its condition in the segment before the `do`, so both pass.
- `literal_assignment` — denies a bare `NAME=value` segment whose name a later segment expands.
  Every allow above and every `settings.json` prefix rule matches the command *text*, so an
  assignment in front of the work makes the call match none of them, and the "don't ask again"
  entry the prompt then offers is that one command with that one value baked in — an approval
  spent on a string nothing will ever match again. The deny names the value, since the whole
  point is that it is a literal and can be written where it is used. It fires only on a value
  that can be: `P=$(…)` is left alone, that being both unwritable inline and the shape
  `secret_paths` relies on to keep a credential out of the transcript, and so is a value built
  from other variables (`f=$D/m$i.img`), which varies with them. The name must actually
  be expanded — an assignment nothing reads is dead (shell state does not survive the call) and
  still rides along as a `vouch` segment. A name set again by another segment — a loop
  counter's `i=$((i+1))`, `((i++))`, `let` — is a variable, not a literal, and passes. An environment prefix is one command word, not a
  segment, so `LANG=C sort` is untouched. Single quotes are not tracked: a `$P` that does not
  expand leaves the assignment dead either way.
- `remote_session` — denies an `ssh`/`sshfs`/`psql`/`mysql`/`mariadb`/`mongosh` bundled with
  anything else: no `;`, `&&`, `||`, `&`, and no unquoted newline. A lone `echo` is not
  company (`… ; echo "rc=$?"` is routine), and neither is a wrapper — `shell::command_word`
  reads through `sudo -u postgres psql` and `timeout 45 ssh`. The client must also lead its
  pipeline, since a producer feeding it rides along on its approval; a consumer after it
  (`| jq`) is fine, and chaining inside the quoted remote command or SQL body is the far
  end's. A heredoc is judged on the text before the marker — the body is data, so the usual
  `psql <<EOF` shape passes, at the cost of not seeing a chain past the terminator.
- `blind_edit` — denies an interpreter heredoc whose body reads a file whole, substitutes into
  the result, and writes it back: the substitution is unverified, so one that matches nothing
  rewrites nothing and reports nothing, leaving a file that looks edited. The Edit tool is the
  alternative named in the deny, its `old_string` mismatch being exactly the check the script
  omits. All three signals or nothing — a script that slurps and substitutes but prints, or that
  iterates a file line by line however much it rewrites, is analysis and passes. That narrowness
  is the point: this is aimed at one habit, not at scripting. Only the heredoc shape is judged;
  `sed -i` and `perl -i` are deliberately out, being deliberate Unix idiom rather than the habit.
  Perl is absent from the interpreter list for the same reason. An intended substitution is
  answered by a waiver over the shared `marker.rs`, spent only against a command this would
  otherwise refuse so an unrelated call cannot consume one.
Four of the denies above are overruled by a one-shot waiver, and `disabled` is a standing
switch over the same `marker.rs`. Every creation is forced to a prompt.
`docs/waivers.md` lists them with the command that creates each — it is written for the user running one by hand, so a new marker goes in it as well as
in its check.

Every check that can *allow* is gathered behind one gate in `dispatch`: a command carrying a
`$( )` or a backtick gets no allow from any of them, and neither does one `shell` cannot scan.
The substitution runs before the program the check classified, so `git log`, `cargo test` or
`glab mr view` in front of it vouches for nothing — each of those checks read the verb and
allowed the call while `$(curl … | sh)` sat in its arguments. Telling an inert `$(pwd)` from a
live one is the classification these checks exist to avoid needing, so the whole shape is
refused and the fold is forfeited with it. Only checks that emit a *deny* look inside a
substitution, `secret_paths` by lifting it out.

`src/checks/shell.rs` is the only shell parser — one mask feeds chain splitting, pipeline
splitting, redirect detection and unquoting. It marks the bytes outside quotes *and* outside
command substitutions, so `grep -rn foo $(pwd) 2>/dev/null` still reads as a search with a
silenced stderr; only a heredoc or an unbalanced quote makes a command unanalyzable. A newline
separates commands like `;` does. A second ad-hoc matcher already caused one bug (a
`2>/dev/null` inside a search *pattern* read as a real redirect), and every check that grew its
own notion of "is this git / glab / a search" grew an evasion with it — ask `shell::program`.

`src/checks/location.rs` answers the other half: where a command runs and where its path
arguments resolve from there. A bare `cd` moves that for every segment behind it, so `dirs`
hands each segment the directory the shell will really be in and every path check reads it from
there — a deny keying on the tool's own cwd reports a correctly-spelled path as wrong the moment
a chain starts with a `cd`.

## Adding a check

Add a module under `src/checks/`, wire it into `dispatch`, and unit-test the pure decision
function. Keep IO (filesystem, env) thin and behind a testable core (see `glab_skill::decide`).
A message the check emits stays a literal beside the branch that emits it until it passes
a kilobyte; past that it moves to a `*.txt` read with `include_str!` (`audit-ask.txt`,
`glab-traps.txt`) and is rewritten by `/compress-messages`, which names what it must not
touch. A deny stays in the transcript for the rest of the session, so one that fires more
than once a session is paid for every time.

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

`./replay.sh <session-id>` replays a past session's design-rationale edits through the built
binary, and `./replay.sh <session-id> <n>` prints the strings the hook was handed for one of
them. Reach for it before writing a probe by hand: an edit inserting a section before an
existing one re-emits that heading, and both the anchor strip and the finding parser were
caught mangling exactly that shape, which no hand-written probe had. Verdicts vary between
runs — the two judge calls race and ollama batches them — so read one replay as a lead and not
as proof.

`./probe-judge.sh <design-rationale.md> <passage.md>...` judges each passage as the added text
of an Edit to that document, `RUNS=` times. Tuning the prompt or the rules is measured with it
over a labelled corpus, never on one passage: every wording that caught a miss here also
started denying passages that had been approved into a real file, and only a set that holds
both kinds shows the trade. The corpus itself is uncommitted, under `probes/`.

License GPL-3.0-only. Commits run `gitleaks git --staged` via `core.hooksPath=githooks`.
