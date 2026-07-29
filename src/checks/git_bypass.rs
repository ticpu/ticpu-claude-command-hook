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

const ALLOW_READ_ONLY: &str =
    "read-only git command (auto-allowed by the hook: no hooks run, nothing mutated)";

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
    match shell::chain_segments(cmd) {
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
            .then(|| {
                deny(
                    cmd.split("<<")
                        .next()
                        .unwrap_or(cmd),
                    cmd,
                    &input.cwd,
                )
            })
            .flatten(),
    }
}

/// A read-only git command mutates nothing and runs no hooks, so it is safe to
/// auto-allow wherever it points — including behind the `cd` Claude Code warns
/// about, since the warning is about hooks from the target directory. Unlike the
/// denies this cannot be decided per segment: an allow ends the permission
/// decision for the *whole* command, so every segment has to be a bare `cd` or a
/// read-only git, or the command goes back to the normal prompt.
pub fn allow_read_only(input: &HookInput) -> Option<HookOutput> {
    let segments = shell::chain_segments(input.command())?;
    let mut git_seen = false;
    for segment in segments {
        if is_bare_cd(segment) {
            continue;
        }
        if !is_read_only_segment(segment) {
            return None;
        }
        git_seen = true;
    }
    git_seen.then(|| HookOutput::allow("PreToolUse", ALLOW_READ_ONLY))
}

/// `cd <path>` and nothing else. A flag, a bare `cd` (to `$HOME`), `cd -`, or a
/// redirect glued to the path (`cd /x>y` truncates `y`) all disqualify it.
fn is_bare_cd(segment: &str) -> bool {
    let mut words = segment.split_whitespace();
    if words.next() != Some("cd") {
        return false;
    }
    let Some(path) = words.next() else {
        return false;
    };
    words
        .next()
        .is_none()
        && !path.starts_with('-')
        && !shell::redirects_stdout(segment)
}

/// `flags` is the part of the command git reads options from; `full` carries the
/// message too, which is where the TDD exemption looks.
fn deny(flags: &str, full: &str, cwd: &str) -> Option<HookOutput> {
    if git_c_path(flags).is_some() && !is_read_only_segment(flags) && points_at_cwd(flags, cwd) {
        return Some(HookOutput::deny("PreToolUse", REDUNDANT_C));
    }
    // Quoted spans are message text, not options: `-m "no --no-verify here"` is a
    // description of the flag, not a use of it. Unbalanced quotes fall back to the
    // raw text so a deny is never lost to a parse failure.
    let flags = shell::unquoted(flags).unwrap_or_else(|| flags.to_string());
    if has_token(&flags, "--no-verify") && !allows_no_verify(full) {
        return Some(HookOutput::deny("PreToolUse", NO_VERIFY));
    }
    if has_token(&flags, "--no-gpg-sign")
        || flags.contains("commit.gpgsign=false")
        || flags.contains("commit.gpgsign=0")
    {
        return Some(HookOutput::deny("PreToolUse", NO_SIGN));
    }
    None
}

/// Provably read-only *as a whole segment*: classifying only the git invocation
/// would let a `| sh` or `> ~/.bashrc` ride along on the allow.
fn is_read_only_segment(segment: &str) -> bool {
    if shell::redirects_stdout(segment) {
        return false;
    }
    let Some(stages) = shell::pipeline_stages(segment) else {
        return false;
    };
    let Some((producer, rest)) = stages.split_first() else {
        return false;
    };
    is_git(producer)
        && is_read_only(producer)
        && rest
            .iter()
            .all(|stage| shell::is_display_only(stage))
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
}

fn parse<'a>(cmd: &'a str) -> Parsed<'a> {
    let mut c_path = None;
    let mut tokens = cmd.split_whitespace();
    let _ = tokens.next(); // "git"
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
            let _ = tokens.next();
            continue;
        }
        if tok.starts_with('-') {
            continue;
        }
        subcommand = Some(tok);
        break;
    }
    Parsed {
        c_path,
        subcommand,
        args: tokens.collect(),
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

fn is_git(cmd: &str) -> bool {
    cmd == "git" || cmd.starts_with("git ")
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
    use super::{allow_read_only, check, is_read_only, parse};
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
        match check(&input).or_else(|| allow_read_only(&input)) {
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
            // A `cd` alone has no git segment to justify the allow.
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
