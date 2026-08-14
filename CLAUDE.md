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
  change, a `stat`, a `test`). A name matches on the basename (a word saying what it holds, a known
  credential dotfile, an `id_` key without `.pub`, a key or keystore extension) or on a directory
  component whose contents are credentials whatever the file inside is called; a source or prose
  extension exempts the *wording* rule alone, so `read_secret_management.rs` reads normally, and
  a directory keeps its meaning. A name that resolves has to exist before it counts — otherwise
  `rg 'aws/credentials' .` refuses the search that quotes its own pattern — while a glob or a
  variable, having nothing to stat, is judged on its wording. `Read` and `Grep` are matched on
  the path they name by the same rules; `Edit`/`Write` are not, a write printing nothing. A
  command `shell` cannot split is judged whole rather than waved through: this is the one check
  with no allow to withhold, so being wrong costs a prompt. A name that reads like a credential
  and is not one is answered by a waiver — `marker.rs`, the same shape the judge's bypass uses,
  under a name carrying none of the words above so that creating it is not itself refused; it is
  named in the deny, prompted on creation and spent by the next refusal. Not caught, deliberately: a
  recursive search rooted at a directory that merely contains one, a `Grep` `glob` (a repo-wide
  `*secret*` is ordinary), and a value captured and later echoed.
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
  stripped — a message describing the rule does not trip it. Last, a `git add` pathspec that
  resolves under the repo root but not under the working directory: it is spelled from the wrong
  root, so the deny names the spelling that works. A bare `cd` earlier in the chain moves that
  working directory first, so every segment is judged where the shell will actually run it —
  otherwise `cd <root> && git add <path-from-root>` reads as misrooted precisely when it is right. These four denies carry a `cwd: … — git repo
  root: …` line, since none of it is answerable from a tool result.
  Bypass flags count only where git reads options: whole
  tokens, outside quotes, before a heredoc marker — so a commit message may name a flag it
  isn't using. `git` is recognized by `shell::program`, so a path, a wrapper or a brace group
  carries the same denies a bare `git` does. **Allows** a command whose every segment is a bare
  `cd`, a lone `echo`, a
  provably read-only git pipeline, or a `git add` naming at least one path that exists as a
  file — a directory, glob or variable stages whatever is under it, which is the sweep the
  blanket forms are denied for. A `cd` carries the allow on its own, with no git segment
  behind it: the working directory persists between Bash calls, so moving it is the work.
  The allowlist
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
- `design_rationale` — a `design-rationale.md` that does not exist yet skips every gate below and
  goes straight to the permission prompt, an absent file being the one read failure that stands the
  check down rather than being reported. Otherwise: an `Edit`/`Write` is refused once before any
  reviewer reads it, carrying the authoring rules and no finding: the writer holds the draft and
  the rules already and applies them only when asked, which is the half of this check that works
  on the passages the model cannot quote a fault in — nine lines on how a timer re-arms, cut to
  three by its own author, twice, where four judged drafts of the same section had produced one
  deny and three passes on wording alone. Keyed on a hash of the text, so a re-issue goes through
  and a redraft is asked once too; that ends when an audit stops changing anything, at one round
  trip per draft and no model call. Behind it, the judged reviews: an edit is reviewed before it lands,
  and the split is the point: `mechanical.rs` decides what can be counted (heading form, section
  length, a CLAUDE.md reference), `judge.rs` asks a local model over ollama for the rest. The
  model reads prose well and counts badly — it passes a forbidden heading form and a section at
  twice the length bound while catching textbook knowledge beside them — so a countable rule
  never moves into the prompt. `rules.md` is the judge's closed list, deliberately shorter than
  CLAUDE.md's authoring rules: given every rule the same model rule-shops until something matches
  and denies nearly everything, so a rule is added only against a labelled set. A finding names a
  rule *number*; the rule's text is looked up from that list afterwards and printed under the
  model's own line, so a number that does not match its quote stays visible — making the model
  restate the rule is more generation under the pressure that invents findings. `overlap.rs` asks
  the second question — does a section already own this decision — in a call of its own, run
  beside the rules one in a `thread::scope`; ollama batches them, so two cost about what one does.
  It is separate because a duplicate is a relation to the rest of the file rather than a fault
  quotable inside the new text, and beside six rules met by quoting a bad passage it is never what
  the model reaches for: on the shared list it missed a near-verbatim clone of an existing section.
  Asked alone it names a section whenever the vocabulary overlaps, so it must also copy out the
  sentence that already says it, and that sentence is checked against that section (whitespace
  collapsed — the file is hard-wrapped) before the deny is emitted. An edit rewriting a section in
  place can't duplicate it, which is why the anchor an append re-emits verbatim is stripped before
  the passage is judged. The whole document plus the edit goes in each prompt with `num_ctx`
  stated explicitly — ollama's default truncates it and the model then answers from the surviving
  fragment with no error. Text under the floor (a deletion, a link fix, a heading rename) never
  reaches the model, and neither does an edit whose new text alone is under it. Any failure —
  unreachable, no verdict, a REVISE naming nothing — allows the edit and says so in a
  `systemMessage`, per review, since a deny nobody can act on costs a rewrite in the dark; an
  objection from one review stands whatever the other did. Trying either review by hand needs a
  passage on some unrelated topic: this repo's own `design-rationale.md` is *about* the judge, so
  a probe written about the judge shares its vocabulary with the sections describing it and a
  verdict on it says nothing — the duplication call has a genuine section to point at, and the
  rules call is reading prose about the rules it is applying. A judged objection denies, so it
  reaches the model, which either revises or argues the finding to the user; an Edit's permission
  prompt renders the diff and nothing else, so an objection carried on an `ask` is one nobody
  reads before deciding. Everything else — a clean verdict, an edit under the floor, a waived one
  — is an `ask`, which prompts however the permission rules read: approving *is* the review. The
  PostToolUse entry says so afterwards and does nothing else, since a writer that is not told
  presents the diff and waits for a second review; a prompt's `permissionDecisionReason` cannot
  carry it, being rendered for whoever answers the prompt — which is why a deny reaches the model
  at all, refusing being the answer — so it travels as `additionalContext`. The rules review
  answers on three levels, since two collapsed "a rule is broken and here it is" together with
  "a rule turns on something I was never told" and called the second a pass: `REVISE` names a
  rule and quotes the passage, `CONTEXT` names what it would have needed to know — is this
  component ours, would anyone face this decision again — and `PASS` is the rest. Both stopping
  levels deny, and both carry the same line telling the writer to take its own passage through
  the CLAUDE.md clauses first; that line is the part that works, the draft and the rules being
  already in its context and unread until something asks for the pass. A `CONTEXT` naming
  nothing passes, the question being the whole of what that level is for, and answers come back
  through a file named in the deny, spent on read like the bypass marker — never through the
  passage, since text added to get past the gate is the padding the gate exists for. The verdict
  word may carry its finding on the same line: told to answer in one word and then to say what
  it needs, the model does both at once, and reading that as no verdict at all passed exactly
  the edits it was meant to stop. A wrong
  finding is overruled by `bypass`, which words the prompt over the shared `marker.rs`: a marker
  under `$XDG_RUNTIME_DIR/claude-hooks/`, named in the
  deny as the command that makes it, consumed by the next judged edit and deleted as it is read
  (before judging, so a failed delete cannot leave a standing waiver). Creating it is forced to a
  prompt so an allowlisted `touch` cannot grant one unseen — but only *creating* it: matching the
  name alone made a `test -e` prompt with a confirmation claiming it granted a waiver, and a
  confirmation that misdescribes itself is worse than none. The countable rules are outside the
  waiver, having nothing to overrule. `MultiEdit` is not matched: Claude Code no longer emits it.
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
- `search_flags` — three denies for flags a grep habit reads wrong. `rg -r` in any form is
  `--replace`, so `rg -rn PAT dir` prints every hit rewritten to `n` and the damage reads as
  ordinary output; `--replace=` is the unambiguous spelling. `rg -h` is `--help`, which prints
  usage and exits 0, so `rg -ohN PAT .` never searches and the usage text lands where the matches
  should be — `-h` alone is exempt, being someone reading the usage, and anything else on the line
  means the search was the point. A pattern beginning with an unescaped `-` is not caught and does
  not need to be: rg rejects it by name and exits 2. And a search filtering another
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

`ticpu-claude-command-hook install` writes those entries itself, matching by binary name so
a re-run after moving the checkout re-points the old entry instead of adding a second one.
It is also the only place the matchers are stated, so a new event or tool in `dispatch`
needs `ENTRIES` in `src/install.rs` widened to match.

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
