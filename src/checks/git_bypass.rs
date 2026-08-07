use std::path::Path;

use crate::checks::shell;
use crate::input::HookInput;
use crate::output::HookOutput;

const NO_VERIFY: &str =
    "`--no-verify` is only allowed for TDD (commit message starts with \"test\"). \
CLAUDE.md forbids skipping git hooks otherwise.";

const NO_SIGN: &str = "Command bypasses git signing (--no-gpg-sign / commit.gpgsign=false). \
CLAUDE.md forbids this unless explicitly requested. If GPG fails on the TTY, run the commit \
manually with `! git commit ...` or fix GPG_TTY.";

const REDUNDANT_C: &str = "`git -C <path>` points at the current working directory — drop the \
`-C` and run the plain `git` command so the normal per-command approval applies. CLAUDE.md: \
avoid `git -C`; use `git <verb>` directly.";

const CD_COMMIT: &str = "Don't `cd` to commit: a commit covers the whole repo, so plain \
`git commit` from the directory you are already in does the same thing and takes the normal \
per-command approval. If the target really is a different repo, work from there or say so and \
let me run it.";

const BLANKET_ADD: &str = "`git add -A` / `.` / `-u` / `*` sweeps untracked scratch into the \
index — CLAUDE.md forbids it (it once staged real PII). Stage explicit paths; `git status` \
first if unsure what is untracked.";

const ALLOW_SAFE: &str = "read-only git, or `git add` on explicit paths (auto-allowed by the hook)";

/// `git add` flags that do not widen the set of files staged. Interactive modes
/// (`-p`, `-i`) would hang the tool, `--pathspec-from-file` takes the paths from a
/// file this cannot see, and the blanket flags are denied outright — all prompt.
const ADD_FLAGS: &[&str] = &[
    "-f",
    "--force",
    "-v",
    "--verbose",
    "-n",
    "--dry-run",
    "-N",
    "--intent-to-add",
    "--renormalize",
    "--sparse",
    "--ignore-removal",
];

/// Pathspecs that mean "whatever is in this tree", untracked files included.
const BLANKET_PATHS: &[&str] = &[".", "./", "..", "../", "*", "*/", ":/", ":/*"];

/// Options that make an otherwise read-only subcommand write a file or run a
/// program: `git diff --output=<path>` writes, `git grep -O<prog>` executes.
/// Their presence disqualifies the command however read-only its verb looks.
const WRITES_OR_RUNS: &[&str] = &[
    "--output",
    "-O",
    "--open-files-in-pager",
    "--ext-cmd",
    "--exec",
];

/// Config values git reads as "off". Keys are case-insensitive, so the whole
/// token is lowercased before comparison.
const FALSY: &[&str] = &["false", "0", "no", "off"];

/// Subcommands that never mutate the repo or worktree in *any* invocation; safe
/// to auto-allow with `-C <path>` regardless of their arguments.
const ALWAYS_READ_ONLY: &[&str] = &[
    "status",
    "log",
    "show",
    "diff",
    "rev-parse",
    "describe",
    "blame",
    "shortlog",
    "ls-files",
    "ls-tree",
    "ls-remote",
    "cat-file",
    "for-each-ref",
    "show-ref",
    "whatchanged",
    "grep",
    "name-rev",
    "merge-base",
    "rev-list",
    "count-objects",
    "var",
    "help",
    "version",
];

/// Denies, per chain segment: a `git` anywhere in a chain carries the same bypass
/// as a bare one, so `cd /x && git commit --no-verify` must not escape.
pub fn check(input: &HookInput) -> Option<HookOutput> {
    let cmd = input.command();
    let flagged = match shell::chain_segments(cmd) {
        Some(segments) => segments
            .into_iter()
            .map(str::trim_start)
            .filter(|segment| is_git(segment))
            .find_map(|segment| deny(segment, segment, &input.cwd)),
        // Unanalyzable (heredoc, substitution): the whole command is the only unit
        // left to judge. Flags are read from the part before the heredoc marker —
        // the body is message text, so a commit message may name `--no-verify` —
        // while the TDD exemption still reads the body it lives in.
        None => mentions_git(cmd)
            .then(|| deny(shell::before_heredoc(cmd), cmd, &input.cwd))
            .flatten(),
    };
    // Last, so a bypass flag in the same command is reported before the cd.
    flagged.or_else(|| cd_before_commit(cmd).then(|| HookOutput::deny("PreToolUse", CD_COMMIT)))
}

/// A `cd` preceding a `git commit` in the same command. Deliberately not routed
/// through `chain_segments`: the shape worth catching is `-m "$(cat <<EOF …)"`,
/// which that parser refuses on principle. Balanced quotes are dropped first, so
/// only operators the shell would act on remain.
fn cd_before_commit(cmd: &str) -> bool {
    let head = shell::before_heredoc(cmd);
    let head = shell::unquoted(head).unwrap_or_else(|| head.to_string());
    let tokens: Vec<&str> = head
        .split_whitespace()
        .collect();
    let mut saw_cd = false;
    for (i, token) in tokens
        .iter()
        .enumerate()
    {
        if !shell::starts_a_command(&tokens, i) {
            continue;
        }
        if *token == "cd" {
            saw_cd = true;
            continue;
        }
        if saw_cd && (*token == "git" || token.ends_with("/git")) {
            let rest = tokens[i..].join(" ");
            if parse(&rest).subcommand == Some("commit") {
                return true;
            }
        }
    }
    false
}

/// Auto-allows the git commands that run no hook of their own, so the `cd` Claude
/// Code warns about — hooks from the target directory — has nothing to warn about.
/// Unlike the denies this cannot be decided per segment: an allow ends the
/// permission decision for the *whole* command, so every segment has to be one of
/// those commands or a segment that does nothing at all (a bare `cd`, an `echo`),
/// or it all goes back to the normal prompt.
pub fn allow_safe(input: &HookInput) -> Option<HookOutput> {
    let segments = shell::chain_segments(input.command())?;
    let mut git_seen = false;
    for segment in segments {
        // A `2>` is not a stdout redirect, but it still truncates whatever path it
        // names — an allow must not cover that.
        if redirects_anything(segment) {
            return None;
        }
        if shell::is_bare_cd(segment) || shell::is_lone_echo(segment) {
            continue;
        }
        if !is_read_only_segment(segment) && !is_explicit_add(segment, &input.cwd) {
            return None;
        }
        git_seen = true;
    }
    git_seen.then(|| HookOutput::allow("PreToolUse", ALLOW_SAFE))
}

/// Any redirect at all. `redirects_stdout` deliberately lets `2>` through — for
/// the fold that is right, since gf still sees stdout — but here the question is
/// whether the command can touch a file, and `2>file` truncates it.
fn redirects_anything(segment: &str) -> bool {
    shell::redirects_stdout(segment)
        || shell::unquoted(segment).is_none_or(|bare| bare.contains('>'))
}

/// `flags` is the part of the command git reads options from; `full` carries the
/// message too, which is where the TDD exemption looks.
fn deny(flags: &str, full: &str, cwd: &str) -> Option<HookOutput> {
    if git_c_path(flags).is_some() && !is_read_only_segment(flags) && points_at_cwd(flags, cwd) {
        return Some(HookOutput::deny("PreToolUse", REDUNDANT_C));
    }
    // Quoted spans are message text, not options: `-m "no --no-verify here"` is a
    // description of the flag, not a use of it. Unbalanced quotes fall back to the
    // raw text so a deny is never lost to a parse failure. A whole token wrapped
    // in quotes is the flag itself — the shell strips those before git reads it —
    // so it is checked against `flags` too, where the span survives as a token.
    let bare = shell::unquoted(flags).unwrap_or_else(|| flags.to_string());
    if (has_option(&bare, "--no-verify") || quotes_an_option(flags, "--no-verify"))
        && !allows_no_verify(full)
    {
        return Some(HookOutput::deny("PreToolUse", NO_VERIFY));
    }
    if has_option(&bare, "--no-gpg-sign")
        || quotes_an_option(flags, "--no-gpg-sign")
        || disables_signing(flags)
    {
        return Some(HookOutput::deny("PreToolUse", NO_SIGN));
    }
    // Pathspecs are read from the raw text: `git add "."` has to keep its quoted
    // token, which the span-deleting unquote would drop entirely.
    if is_blanket_add(flags) {
        return Some(HookOutput::deny("PreToolUse", BLANKET_ADD));
    }
    None
}

/// `--no-verify`, or the `-n` that means it. `-n` counts only on `commit`: on
/// `push` and `merge` the same letter is a different option entirely.
fn has_option(cmd: &str, flag: &str) -> bool {
    if has_token(cmd, flag) {
        return true;
    }
    if flag != "--no-verify" {
        return false;
    }
    let p = parse(cmd);
    p.subcommand == Some("commit")
        && p.args
            .iter()
            .any(|arg| arg.starts_with('-') && !arg.starts_with("--") && arg.contains('n'))
}

/// A whole token wrapped in quotes and nothing else. A message *containing* the
/// flag splits into several tokens, none of which is the flag fully quoted.
fn quotes_an_option(cmd: &str, flag: &str) -> bool {
    cmd.split_whitespace()
        .any(|word| {
            word.len() > flag.len()
                && matches!(
                    word.as_bytes()
                        .first(),
                    Some(b'"') | Some(b'\'')
                )
                && unquote(word) == flag
        })
}

/// `-c commit.gpgsign=<falsy>` in any spelling git accepts: the key is
/// case-insensitive and the value has four off forms. The token has to match
/// exactly, so a commit message mentioning the setting is not a use of it.
fn disables_signing(cmd: &str) -> bool {
    cmd.split_whitespace()
        .any(|word| {
            let word = unquote(word).to_ascii_lowercase();
            word.strip_prefix("commit.gpgsign=")
                .is_some_and(|value| FALSY.contains(&value))
        })
}

/// The git invocation a segment produces its output with — but only when the
/// segment can be judged by that invocation alone. A stdout redirect or a consumer
/// that is not display-only (`| sh`, `| xargs rm`) would otherwise ride along on
/// whatever the invocation is allowed.
fn git_producer(segment: &str) -> Option<&str> {
    if shell::redirects_stdout(segment) {
        return None;
    }
    let stages = shell::pipeline_stages(segment)?;
    let (producer, rest) = stages.split_first()?;
    (is_git(producer)
        && rest
            .iter()
            .all(|stage| is_harmless_consumer(stage)))
    .then_some(*producer)
}

/// A later stage that writes nothing and runs nothing. Weaker than `grep_fold`'s
/// display-only test, which additionally has to survive gf's folding — here the
/// only question is whether the stage adds a side effect to the git command's.
fn is_harmless_consumer(stage: &str) -> bool {
    if shell::is_display_only(stage) {
        return true;
    }
    match shell::command_word(stage) {
        Some("wc") => true,
        Some("sed") => prints_line_ranges(stage),
        _ => false,
    }
}

/// `sed` restricted to selecting lines: every argument is the quiet flag, a bare
/// `-e`, or a script built only from line numbers and `p`/`q`/`d`. That leaves out
/// `-i`, an `s///w` or `w` command and GNU's `e`, so nothing is written or run. A
/// glued `-e<script>`/`--expression=` is not accepted, since the script would ride
/// along unchecked.
fn prints_line_ranges(stage: &str) -> bool {
    let mut args = stage.split_whitespace();
    let _ = args.next(); // "sed"
    let mut scripts = 0;
    for arg in args {
        if matches!(arg, "-n" | "--quiet" | "--silent" | "-e") {
            continue;
        }
        let script = unquote(arg);
        if script.is_empty()
            || !script
                .chars()
                .all(|c| c.is_ascii_digit() || ",;p$qd".contains(c))
        {
            return false;
        }
        scripts += 1;
    }
    scripts > 0
}

fn is_read_only_segment(segment: &str) -> bool {
    git_producer(segment).is_some_and(is_read_only)
}

fn is_explicit_add(segment: &str, cwd: &str) -> bool {
    git_producer(segment).is_some_and(|cmd| adds_explicit_paths(cmd, cwd))
}

/// `git add` naming at least one path, with no flag that widens what gets staged.
/// Staging named files runs no hook and `git restore --staged` undoes it, so the
/// prompt has nothing to protect; the blanket forms are denied instead.
///
/// Every pathspec has to resolve to an existing regular file. A directory, a
/// glob or a variable stages whatever happens to be under it — untracked scratch
/// included — which is the same sweep `git add .` is denied for. Staging a
/// deletion names a path that no longer exists and so falls through to the
/// normal prompt.
fn adds_explicit_paths(cmd: &str, cwd: &str) -> bool {
    let p = parse(cmd);
    if p.subcommand != Some("add") {
        return false;
    }
    let mut paths = 0;
    for arg in &p.args {
        if *arg == "--" {
            continue;
        }
        if arg.starts_with('-') {
            if !ADD_FLAGS.contains(arg) {
                return false;
            }
            continue;
        }
        let path = unquote(arg);
        if BLANKET_PATHS.contains(&path) || !names_a_file(path, cwd) {
            return false;
        }
        paths += 1;
    }
    paths > 0
}

fn names_a_file(path: &str, cwd: &str) -> bool {
    if path.contains(['*', '?', '[', '$', '~']) {
        return false;
    }
    let path = if Path::new(path).is_absolute() {
        Path::new(path).to_path_buf()
    } else {
        Path::new(cwd).join(path)
    };
    path.is_file()
}

/// `-A`/`--all`/`-u`/`--update` — including inside a short bundle like `-Av` — or a
/// pathspec standing for the whole tree. Quotes are stripped per token, since
/// `git add "."` reaches git as the same pathspec the bare form does.
fn is_blanket_add(cmd: &str) -> bool {
    let p = parse(cmd);
    if p.subcommand != Some("add") {
        return false;
    }
    p.args
        .iter()
        .map(|arg| unquote(arg))
        .any(|arg| {
            BLANKET_PATHS.contains(&arg)
                || matches!(arg, "--all" | "--update")
                || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains(['A', 'u']))
        })
}

fn has_token(segment: &str, flag: &str) -> bool {
    segment
        .split_whitespace()
        .any(|word| word == flag)
}

fn mentions_git(cmd: &str) -> bool {
    cmd.split_whitespace()
        .any(|word| word == "git" || word.ends_with("/git"))
}

/// True when `git -C <path>` targets the same directory the tool already runs in,
/// making the `-C` redundant. Both sides are canonicalized so symlinked mount
/// paths and `.`/trailing-slash forms compare equal; if either fails to resolve
/// we fall back to a literal compare rather than guessing.
fn points_at_cwd(cmd: &str, cwd: &str) -> bool {
    let Some(target) = git_c_path(cmd) else {
        return false;
    };
    if cwd.is_empty() {
        return false;
    }
    let target = if Path::new(target).is_absolute() {
        Path::new(target).to_path_buf()
    } else {
        Path::new(cwd).join(target)
    };
    let cwd = Path::new(cwd);
    match (target.canonicalize(), cwd.canonicalize()) {
        (Ok(t), Ok(c)) => t == c,
        _ => target == cwd,
    }
}

/// Walk the global-option prefix once, capturing the `-C` argument (if any) and
/// the subcommand token that terminates the prefix. Everything before the first
/// bare (non-`-`) token is a git global option; a `-C` after the subcommand is an
/// argument to that subcommand, not a working-directory change.
struct Parsed<'a> {
    c_path: Option<&'a str>,
    subcommand: Option<&'a str>,
    /// Tokens following the subcommand — the subcommand's own args/flags.
    args: Vec<&'a str>,
    /// A `-c`/`--config-env` was given. Config can point at a program to run
    /// (a pager, an external diff, an ssh command), so no invocation carrying one
    /// is classified read-only.
    sets_config: bool,
}

fn parse<'a>(cmd: &'a str) -> Parsed<'a> {
    let mut c_path = None;
    let mut sets_config = false;
    let mut tokens = cmd.split_whitespace();
    // Step over everything up to and including the `git` token itself, so a path
    // or a wrapper (`/usr/bin/git`, `sudo git`) parses like a bare `git`.
    for token in tokens.by_ref() {
        if token
            .rsplit('/')
            .next()
            == Some("git")
        {
            break;
        }
    }
    let mut subcommand = None;
    while let Some(tok) = tokens.next() {
        if let Some(rest) = tok.strip_prefix("-C") {
            let raw = if let Some(eq) = rest.strip_prefix('=') {
                Some(eq)
            } else if !rest.is_empty() {
                Some(rest)
            } else {
                tokens.next()
            };
            c_path = raw.map(unquote);
            continue;
        }
        // `-c key=val`, `--git-dir=...` etc. consume their own value inline or as
        // a following token; the only one we must not misread as a subcommand is
        // the two-token `-c val` form.
        if tok == "-c" || tok == "--git-dir" || tok == "--work-tree" || tok == "--namespace" {
            sets_config |= tok == "-c";
            let _ = tokens.next();
            continue;
        }
        if tok.starts_with('-') {
            sets_config |= tok.starts_with("-c") || tok.starts_with("--config-env");
            continue;
        }
        subcommand = Some(tok);
        break;
    }
    Parsed {
        c_path,
        subcommand,
        args: tokens.collect(),
        sets_config,
    }
}

fn git_c_path(cmd: &str) -> Option<&str> {
    parse(cmd).c_path
}

/// Fail-safe classifier: returns true only when the command is *provably*
/// read-only. Subcommands in `ALWAYS_READ_ONLY` qualify unconditionally; the
/// mode-dependent ones (branch/tag/config/remote/reflog/symbolic-ref) qualify
/// only in explicitly-whitelisted read-only forms. Everything else — including
/// any unrecognized flag or subcommand — returns false and falls through to the
/// normal permission prompt, so an unforeseen write mode is never auto-allowed.
fn is_read_only(cmd: &str) -> bool {
    let p = parse(cmd);
    let Some(sub) = p.subcommand else {
        return false;
    };
    if p.sets_config
        || p.args
            .iter()
            .any(|arg| writes_or_runs(arg))
    {
        return false;
    }
    if ALWAYS_READ_ONLY.contains(&sub) {
        return true;
    }
    match sub {
        // Listing is the only read-only mode. Every token must be a whitelisted
        // read-only flag (or the value of a value-taking one); a bare positional
        // means create/rename/set and disqualifies the command.
        "branch" | "tag" => args_all_read_only(
            &p.args,
            &[
                "--list",
                "-l",
                "--show-current",
                "-a",
                "--all",
                "-r",
                "--remotes",
                "-v",
                "-vv",
                "--verbose",
                "--no-contains",
                "--no-merged",
                "-i",
                "--ignore-case",
                "--color",
                "--no-color",
                "--column",
                "--no-column",
            ],
            &[
                "--contains",
                "--merged",
                "--points-at",
                "--sort",
                "--format",
            ],
        ),
        // Only the query verbs read; add/remove/set/rename/prune write.
        "remote" => match p
            .args
            .split_first()
        {
            None => true,
            Some((&"-v", rest)) | Some((&"--verbose", rest)) => rest.is_empty(),
            Some((&"show", _)) | Some((&"get-url", _)) => true,
            _ => false,
        },
        // Only the read verbs / --get* / --list queries.
        "config" => config_is_read(&p.args),
        // `git reflog` / `reflog show` read; expire/delete write.
        "reflog" => matches!(
            p.args
                .first(),
            None | Some(&"show")
        ),
        // One-arg form (`symbolic-ref HEAD`) reads; two-arg form sets it.
        "symbolic-ref" => {
            p.args
                .iter()
                .filter(|a| !a.starts_with('-'))
                .count()
                <= 1
        }
        _ => false,
    }
}

/// True only if every token is a whitelisted read-only flag. `nullary` flags
/// take no value; `unary` flags consume the following token as their value
/// (unless given as `--flag=value`). Any token that is neither — a bare
/// positional or an unrecognized flag — disqualifies the command (fail-safe).
fn args_all_read_only(args: &[&str], nullary: &[&str], unary: &[&str]) -> bool {
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        let name = a
            .split('=')
            .next()
            .unwrap_or(a);
        if nullary.contains(&name) {
            continue;
        }
        if unary.contains(&name) {
            // `--flag=value` carries its value inline; `--flag value` eats next.
            if !a.contains('=') {
                let _ = it.next();
            }
            continue;
        }
        return false;
    }
    true
}

/// `git config` reads only via an explicit query verb/flag, or a single lone key
/// with no value. Anything else — a second positional (the value), an edit/unset
/// flag, or the `set`/`unset` subcommand form — is treated as a write.
fn config_is_read(args: &[&str]) -> bool {
    match args.split_first() {
        None => false,
        Some((first, rest)) => match *first {
            "get" | "list" | "--get" | "--get-all" | "--get-regexp" | "--get-urlmatch" | "-l"
            | "--list" | "--get-color" | "--get-colorbool" => true,
            // `config <key>` with nothing else reads that key.
            key if !key.starts_with('-') && rest.is_empty() => true,
            // A leading flag like `--global`/`--local` followed by a query verb.
            flag if flag.starts_with('-') && is_config_scope(flag) => config_is_read(rest),
            _ => false,
        },
    }
}

fn is_config_scope(flag: &str) -> bool {
    matches!(flag, "--global" | "--local" | "--system" | "--worktree")
}

fn unquote(tok: &str) -> &str {
    tok.strip_prefix(['"', '\''])
        .and_then(|s| s.strip_suffix(['"', '\'']))
        .unwrap_or(tok)
}

/// The stage runs git, however it is reached — a path, a wrapper, a brace group.
fn is_git(cmd: &str) -> bool {
    shell::program(cmd) == Some("git")
}

/// An option that makes the command write a file or execute a program. The name
/// is taken before any `=`, and `-O` is matched as a prefix since its value glues
/// on (`-Oless`).
fn writes_or_runs(arg: &str) -> bool {
    let name = arg
        .split('=')
        .next()
        .unwrap_or(arg);
    WRITES_OR_RUNS.contains(&name) || arg.starts_with("-O")
}

/// TDD escape hatch: a commit whose message starts with "test", supplied either
/// inline via `-m` or through a heredoc body.
fn allows_no_verify(cmd: &str) -> bool {
    let mut rest = cmd;
    while let Some(pos) = rest.find("-m") {
        let after = rest[pos + 2..].trim_start();
        let after = after
            .strip_prefix(['"', '\''])
            .unwrap_or(after);
        if after.starts_with("test") {
            return true;
        }
        rest = &rest[pos + 2..];
    }
    cmd.contains("<<")
        && cmd
            .lines()
            .any(|l| {
                l.trim_start()
                    .starts_with("test")
            })
}

#[cfg(test)]
mod tests {
    use super::{allow_safe, check, is_read_only, parse};
    use crate::input::HookInput;

    fn input(cmd: &str, cwd: &str) -> HookInput {
        HookInput {
            hook_event_name: "PreToolUse".to_string(),
            tool_name: "Bash".to_string(),
            cwd: cwd.to_string(),
            tool_input: serde_json::json!({ "command": cmd }),
            ..Default::default()
        }
    }

    /// Decision as one of: allow / deny / prompt (None). Both entry points, in the
    /// order `dispatch` runs them — a deny has to beat the allow.
    fn decision(cmd: &str, cwd: &str) -> &'static str {
        let input = input(cmd, cwd);
        match check(&input).or_else(|| allow_safe(&input)) {
            None => "prompt",
            Some(out) => match out
                .hook_specific_output
                .and_then(|h| h.permission_decision)
                .as_deref()
            {
                Some("allow") => "allow",
                Some("deny") => "deny",
                other => panic!("unexpected decision {other:?}"),
            },
        }
    }

    fn blocked(cmd: &str) -> bool {
        decision(cmd, "") == "deny"
    }

    #[test]
    fn blocks_bypasses() {
        assert!(blocked("git commit --no-verify -m \"feat: x\""));
        assert!(blocked("git commit --no-gpg-sign -m \"feat: x\""));
        assert!(blocked("git -c commit.gpgsign=false commit -m \"x\""));
        assert!(blocked("git -c commit.gpgsign=0 commit -m \"x\""));
    }

    /// git is git however it is reached; the deny cannot key on the literal word
    /// starting the segment.
    #[test]
    fn git_reached_indirectly_is_still_git() {
        for cmd in [
            "/usr/bin/git commit --no-verify -m \"feat: x\"",
            "sudo git commit --no-verify -m \"feat: x\"",
            "env git commit --no-verify -m \"feat: x\"",
            "{ git commit --no-verify -m \"feat: x\"; }",
            "sudo git add -A",
            "{ git add -A; }",
            "/usr/bin/git add .",
        ] {
            assert!(blocked(cmd), "{cmd}");
        }
    }

    /// The shell strips quotes before git reads the option, and git reads its
    /// config keys case-insensitively with four spellings of "off".
    #[test]
    fn quoted_and_spelled_out_bypasses() {
        for cmd in [
            "git commit \"--no-verify\" -m \"feat: x\"",
            "git commit '--no-verify' -m \"feat: x\"",
            "git commit -n -m \"feat: x\"",
            "git commit -an -m \"feat: x\"",
            "git -c 'commit.gpgsign=false' commit -m \"feat: x\"",
            "git -c \"commit.gpgsign=false\" commit -m \"feat: x\"",
            "git -c commit.gpgSign=false commit -m \"feat: x\"",
            "git -c commit.gpgsign=no commit -m \"feat: x\"",
            "git -c commit.gpgsign=off commit -m \"feat: x\"",
        ] {
            assert!(blocked(cmd), "{cmd}");
        }
    }

    /// `-n` is `--no-verify` only where git spells it that way.
    #[test]
    fn short_n_elsewhere_is_a_different_option() {
        for cmd in [
            // --dry-run
            "git push -n origin master",
            // --no-stat
            "git merge -n topic",
            "git clean -n",
        ] {
            assert!(!blocked(cmd), "{cmd}");
        }
    }

    /// A read-only verb stops being read-only when an option writes a file or
    /// runs a program, and `-c` can point config at either.
    #[test]
    fn read_only_verbs_with_a_writing_option_prompt() {
        for cmd in [
            "git diff --output=/x/pwned",
            "git log --output=/x/pwned --oneline",
            "git show --output=/x/pwned HEAD",
            "git grep --open-files-in-pager=rm -n foo",
            "git grep -Orm -n foo",
            "git -c core.pager=rm log",
            "git -c core.sshCommand=rm ls-remote",
            "git --config-env=core.pager=EVIL log",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    /// `2>` is not a stdout redirect, but it still truncates the file it names.
    #[test]
    fn a_stderr_redirect_is_not_covered_by_the_allow() {
        for cmd in [
            "git log 2>/home/jerome.poulin/.bashrc",
            "git -C /x status 2>&-",
            "cd /x && git status 2>errs",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    #[test]
    fn allows_tdd_and_clean() {
        assert!(!blocked("git commit --no-verify -m \"test: red\""));
        assert!(!blocked("git commit --no-verify -m 'test(scope): red'"));
        assert!(!blocked("git commit -m \"feat: x\""));
        assert_eq!(decision("git status", ""), "allow");
        assert_eq!(decision("git push", ""), "prompt");
        assert!(!blocked("cargo test --no-verify-something"));
    }

    #[test]
    fn heredoc_test_message_allowed() {
        let cmd = "git commit --no-verify -F - <<EOF\ntest: red bar\nEOF";
        assert!(!blocked(cmd));
    }

    /// The reported stall: a subagent inspecting another directory of the project
    /// it is already in.
    #[test]
    fn read_only_git_behind_a_cd_auto_allowed() {
        for cmd in [
            "cd /x && git diff --stat Cargo.lock",
            "cd /x && git status && git log --oneline",
            "cd /x && git log | head -20",
            "cd /x; git show HEAD",
            "cd /x && git -C /y log",
            "cd ~/GIT/eido && git status",
        ] {
            assert_eq!(decision(cmd, "/here"), "allow", "{cmd}");
        }
    }

    /// Quoting citation ranges out of two files, with labels between them.
    #[test]
    fn a_labelled_multi_file_quote_auto_allowed() {
        let cmd = "cd /x/freeswitch && git show c2c5964:src/switch_utils.c | \
                   sed -n '2766,2770p;2796,2800p' && echo '=== amr 688-718' && \
                   git show c2c5964:src/mod/codecs/mod_amr/mod_amr.c | sed -n '688,692p;712,718p'";
        assert_eq!(decision(cmd, "/here"), "allow");
    }

    #[test]
    fn only_side_effect_free_consumers_qualify() {
        for cmd in [
            "git show HEAD:a.c | wc -l",
            "cd /x && git log | sed -n '1,20p'",
            "git show HEAD:a.c | sed -n -e '1,5p'",
        ] {
            assert_eq!(decision(cmd, "/here"), "allow", "{cmd}");
        }
        for cmd in [
            // Writes: in place, via a `w` command, via a glued script.
            "git show HEAD:a.c | sed -i '1d'",
            "git show HEAD:a.c | sed -n '1,5p;w /x/out'",
            "git show HEAD:a.c | sed -n -e'1,5p;w /x/out'",
            // Substitution and GNU `e` execute or rewrite.
            "git show HEAD:a.c | sed 's/a/b/'",
            "git show HEAD:a.c | sed -n '1e rm -rf /x'",
            // Not a consumer at all: sed reads the file itself.
            "git show HEAD:a.c | sed -n '1,5p' other.c",
            // echo is harmless alone, not as a producer or with a redirect.
            "echo pwned > /x/f && git status",
            "echo rm -rf | sh && git status",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    /// A commit is repo-wide, so the `cd` buys nothing and runs the target repo's
    /// hooks — the one case Claude Code's warning is literally about.
    #[test]
    fn a_cd_before_commit_is_denied() {
        for cmd in [
            "cd /x && git commit -m 'feat: y'",
            "cd /x; git commit -m y",
            "cd /x && git add a.rs && git commit -m y",
            "cd /x && git -c user.name=Y commit -m y",
            "cd /x && /usr/bin/git commit -m y",
            // The reported shape: substitution *and* heredoc, which `shell` refuses.
            "cd /x/rust && git commit -m \"$(cat <<'EOF'\nrefactor: collapse the rule\nEOF\n)\"",
        ] {
            assert_eq!(decision(cmd, "/here"), "deny", "{cmd}");
        }
    }

    /// The rule keys on a real `cd`, not a description of one.
    #[test]
    fn talking_about_the_cd_is_not_doing_it() {
        for cmd in [
            "git commit -m \"fix: deny cd && git commit\"",
            "git commit -F - <<EOF\nfix: deny `cd /x && git commit`\nEOF",
            "cd /x && git status",
            "git -C /x commit -m y",
        ] {
            assert_ne!(decision(cmd, "/here"), "deny", "{cmd}");
        }
    }

    /// Staging named paths runs no hook, so the `cd` warning has nothing to add and
    /// the allowlist entry it overrode (`Bash(git add:*)`) applies again. The paths
    /// have to exist, so these name real files in this repo.
    #[test]
    fn staging_explicit_paths_auto_allowed() {
        let root = env!("CARGO_MANIFEST_DIR");
        for cmd in [
            "git add src/checks/git_bypass.rs",
            "git add src/checks/git_bypass.rs src/checks/shell.rs",
            "cd /x && git add src/main.rs",
            "cd /x && git status && git add Cargo.toml",
            "git add -- README.md CLAUDE.md",
            "git add -f src/checks/mod.rs",
        ] {
            assert_eq!(decision(cmd, root), "allow", "{cmd}");
        }
    }

    /// A pathspec that is not one named file sweeps whatever is under it — the
    /// same staging `git add .` is denied for, so it does not get the auto-allow.
    #[test]
    fn a_pathspec_that_is_not_a_named_file_prompts() {
        let root = env!("CARGO_MANIFEST_DIR");
        for cmd in [
            // A directory stages every untracked file inside it.
            "git add src",
            "git add src/checks",
            // Globs and variables expand to whatever happens to be there.
            "git add src/*",
            "git add *.rs",
            "git add \"$PWD\"",
            "git add $HOME/x.rs",
            // Absolute path to the repo root is `git add .` spelled long.
            "git add /home/jerome.poulin/GIT/ticpu-claude-command-hook",
            // Staging a deletion: the path is gone, so it takes the normal prompt.
            "git add src/checks/removed.rs",
        ] {
            assert_eq!(decision(cmd, root), "prompt", "{cmd}");
        }
    }

    /// Quoting a blanket pathspec changes nothing for git, so it must not change
    /// the verdict here either.
    #[test]
    fn quoted_blanket_staging_denied() {
        let root = env!("CARGO_MANIFEST_DIR");
        for cmd in [
            "git add \".\"",
            "git add '.'",
            "git add '*'",
            "git add \"*\"",
            "git add ':/'",
            "git add \"-A\"",
        ] {
            assert_eq!(decision(cmd, root), "deny", "{cmd}");
        }
    }

    /// The forms that swept untracked scratch into a commit once already.
    #[test]
    fn blanket_staging_denied() {
        for cmd in [
            "git add -A",
            "git add .",
            "git add -u",
            "git add --all",
            "git add --update",
            "cd /x && git add -Av",
            "git add *",
            "git add ..",
            "git add src/a.rs .",
        ] {
            assert_eq!(decision(cmd, "/here"), "deny", "{cmd}");
        }
    }

    /// Anything the classifier cannot vouch for keeps the prompt.
    #[test]
    fn other_add_forms_prompt() {
        for cmd in [
            // Interactive: it would hang the tool.
            "git add -p src/a.rs",
            "git add -i",
            // The paths come from a file this cannot read.
            "git add --pathspec-from-file=list",
            // No path named at all.
            "cd /x && git add",
            // A consumer that is not display-only, and a stdout redirect.
            "git add a.rs | sh",
            "git add a.rs > out",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    /// A `cd` that is not just a directory change, or a write anywhere in the
    /// chain, drops the whole command back to the normal prompt.
    #[test]
    fn a_cd_chain_with_anything_else_prompts() {
        for cmd in [
            "cd /x && git stash pop",
            "cd /x && git log; cargo build",
            "cd /x && git log && rm -rf /y",
            "cd /x && git log | sh",
            "cd /x && git log > out",
            "cd && git log",
            "cd - && git log",
            "cd /x>y && git log",
            "cd /x /y && git log",
            "pushd /x && git log",
            // A `cd` alone has no git segment to justify the allow; `lone_cd` denies it.
            "cd /x",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    /// An allow decides the whole command, so a piped or redirected consumer must
    /// not ride along on the read-only git that precedes it.
    #[test]
    fn a_consumer_of_the_output_is_not_covered() {
        for cmd in [
            "git -C /x status; rm -rf /y",
            "git -C /x log | sh",
            "git -C /x log > ~/.bashrc",
            "git -C /x log | xargs rm",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    /// A bypass flag on any segment of a chain, not just a bare `git` command.
    #[test]
    fn bypasses_in_a_chain_are_denied() {
        assert_eq!(
            decision("cd /x && git commit --no-verify -m 'feat: x'", "/here"),
            "deny"
        );
        assert_eq!(
            decision("echo x; git commit --no-gpg-sign -m y", "/here"),
            "deny"
        );
        // Unanalyzable heredoc: the whole command is the only unit left to judge.
        let heredoc = "echo x; git commit --no-verify -F - <<EOF\nfeat: x\nEOF";
        assert_eq!(decision(heredoc, "/here"), "deny");
        let tdd = "echo x; git commit --no-verify -F - <<EOF\ntest: red\nEOF";
        assert_eq!(decision(tdd, "/here"), "prompt");
    }

    /// A commit message is allowed to talk about the flags. They only count where
    /// git reads options: outside quotes, before a heredoc body.
    #[test]
    fn a_message_naming_the_flag_is_not_a_bypass() {
        for cmd in [
            "git commit -m \"fix: deny --no-verify in a chain\"",
            "git commit -F - <<EOF\nfix: deny --no-verify in a chain\nEOF",
            "git commit -m 'drop --no-gpg-sign handling'",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    #[test]
    fn read_only_dash_c_auto_allowed() {
        for cmd in [
            "git -C /some/other/repo status",
            "git -C=/other log --oneline",
            "git -C /x diff HEAD~1",
            "git -C /x show abc123",
            "git -C /x branch --list",
            "git -C /x remote -v",
            "git -C /x config --get user.email",
            "git -C /x config user.email",
            "git -C /x rev-parse HEAD",
        ] {
            assert_eq!(decision(cmd, "/here"), "allow", "{cmd}");
        }
    }

    #[test]
    fn write_dash_c_not_auto_allowed() {
        // Not read-only → falls through to prompt unless it targets cwd.
        for cmd in [
            "git -C /other commit -m x",
            "git -C /other branch -d foo",
            "git -C /other branch newbranch",
            "git -C /other tag -d v1",
            "git -C /other tag -a v1 -m msg",
            "git -C /other tag v1",
            "git -C /other config user.email a@b",
            "git -C /other config set user.email a@b",
            "git -C /other config --unset user.email",
            "git -C /other remote add o url",
            "git -C /other remote set-url o url",
            "git -C /other reflog expire --all",
            "git -C /other symbolic-ref HEAD refs/heads/x",
            "git -C /other push",
            "git -C /other checkout main",
        ] {
            assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
        }
    }

    #[test]
    fn redundant_dash_c_at_cwd_denied() {
        // Non-read-only -C pointing at the current dir is the redundant case.
        let cwd = std::env::current_dir().unwrap();
        let cwd = cwd
            .to_str()
            .unwrap();
        let cmd = format!("git -C {cwd} commit -m x");
        assert_eq!(decision(&cmd, cwd), "deny");
        // "." resolves to cwd too.
        assert_eq!(decision("git -C . push", cwd), "deny");
    }

    #[test]
    fn dash_c_after_subcommand_is_not_workdir() {
        // `git branch -C old new` renames; the -C is a branch flag, no c_path.
        assert!(parse("git branch -C old new")
            .c_path
            .is_none());
    }

    #[test]
    fn is_read_only_classifies() {
        assert!(is_read_only("git -C /x log"));
        assert!(is_read_only("git -C /x symbolic-ref HEAD"));
        assert!(is_read_only("git -C /x config --list"));
        assert!(is_read_only("git -C /x config --global --get user.email"));
        assert!(is_read_only("git -C /x config user.email"));
        assert!(is_read_only("git -C /x branch --list --contains HEAD"));
        assert!(is_read_only("git -C /x remote"));

        assert!(!is_read_only("git -C /x commit -m x"));
        assert!(!is_read_only("git -C /x config user.name Jerome"));
        assert!(!is_read_only("git -C /x config set user.name Jerome"));
        assert!(!is_read_only("git -C /x symbolic-ref HEAD refs/heads/x"));
        assert!(!is_read_only("git -C /x branch newbranch"));
        assert!(!is_read_only("git -C /x remote add o url"));
        // Unknown flag on an otherwise-listing subcommand → not provably safe.
        assert!(!is_read_only("git -C /x branch --some-new-flag"));
    }
}
