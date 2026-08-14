mod add;
mod location;
mod parse;
mod read_only;
#[cfg(test)]
mod tests;

use crate::checks::git_bypass::add::{is_blanket_add, is_explicit_add, misrooted_paths};
use crate::checks::git_bypass::location::{hint, points_at_cwd, resolve};
use crate::checks::git_bypass::parse::{git_c_path, has_token, is_git, mentions_git, parse};
use crate::checks::git_bypass::read_only::is_read_only_segment;
use crate::checks::shell;
use crate::checks::shell::unquote_token;
use crate::input::HookInput;
use crate::output::HookOutput;

const NO_VERIFY: &str = "`--no-verify` is only allowed for TDD (commit message starts with \"test\"). \
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

const MISROOTED_ADD: &str = "Pathspec spelled from the repo root, not from here: ";

const ALLOW_SAFE: &str =
    "a bare `cd`, read-only git, or `git add` on explicit paths (auto-allowed by the hook)";

/// Config values git reads as "off". Keys are case-insensitive, so the whole
/// token is lowercased before comparison.
const FALSY: &[&str] = &["false", "0", "no", "off"];

/// Denies, per chain segment: a `git` anywhere in a chain carries the same bypass
/// as a bare one, so `cd /x && git commit --no-verify` must not escape.
pub fn check(input: &HookInput) -> Option<HookOutput> {
    let cmd = input.command();
    let flagged = match shell::chain_segments(cmd) {
        Some(segments) => walk(segments, &input.cwd, |segment, cwd| {
            is_git(segment)
                .then(|| deny(segment, segment, cwd))
                .flatten()
        }),
        // Unanalyzable (heredoc, substitution): the whole command is the only unit
        // left to judge. Flags are read from the part before the heredoc marker —
        // the body is message text, so a commit message may name `--no-verify` —
        // while the TDD exemption still reads the body it lives in.
        None => mentions_git(cmd)
            .then(|| deny(shell::before_heredoc(cmd), cmd, &input.cwd))
            .flatten(),
    };
    // Last, so a bypass flag in the same command is reported before the cd.
    flagged.or_else(|| cd_before_commit(cmd).then(|| located(CD_COMMIT, &input.cwd)))
}

/// Segments in order, each judged against the directory it will actually run in:
/// a bare `cd` moves that directory for everything after it, so a path argument
/// in a later segment resolves from there and not from where the tool started.
fn walk<T>(
    segments: Vec<&str>,
    cwd: &str,
    mut judge: impl FnMut(&str, &str) -> Option<T>,
) -> Option<T> {
    let mut here = cwd.to_string();
    for segment in segments {
        let segment = segment.trim_start();
        if let Some(target) = shell::bare_cd_target(segment) {
            here = resolve(unquote_token(target), &here)
                .display()
                .to_string();
            continue;
        }
        if let Some(found) = judge(segment, &here) {
            return Some(found);
        }
    }
    None
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
    let mut cd_seen = false;
    let mut here = input
        .cwd
        .clone();
    for segment in segments {
        // A `2>` is not a stdout redirect, but it still truncates whatever path it
        // names — an allow must not cover that.
        if redirects_anything(segment) {
            return None;
        }
        if let Some(target) = shell::bare_cd_target(segment) {
            here = resolve(unquote_token(target), &here)
                .display()
                .to_string();
            cd_seen = true;
            continue;
        }
        if shell::is_lone_echo(segment) {
            continue;
        }
        if !is_read_only_segment(segment) && !is_explicit_add(segment, &here) {
            return None;
        }
        git_seen = true;
    }
    // A `cd` earns the allow on its own: the working directory persists between Bash
    // calls, so moving it is the work, and nothing else is left behind.
    (git_seen || cd_seen).then(|| HookOutput::allow("PreToolUse", ALLOW_SAFE))
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
        return Some(located(REDUNDANT_C, cwd));
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
        return Some(located(BLANKET_ADD, cwd));
    }
    let misrooted = misrooted_paths(flags, cwd);
    if !misrooted.is_empty() {
        let pairs: Vec<String> = misrooted
            .iter()
            .map(|(given, corrected)| format!("`{given}` → `{corrected}`"))
            .collect();
        return Some(located(
            &format!("{MISROOTED_ADD}{}.", pairs.join(", ")),
            cwd,
        ));
    }
    None
}

/// A deny that only makes sense once the reader knows where the command runs.
fn located(reason: &str, cwd: &str) -> HookOutput {
    HookOutput::deny("PreToolUse", &format!("{reason}{}", hint(cwd)))
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
                && unquote_token(word) == flag
        })
}

/// `-c commit.gpgsign=<falsy>` in any spelling git accepts: the key is
/// case-insensitive and the value has four off forms. The token has to match
/// exactly, so a commit message mentioning the setting is not a use of it.
fn disables_signing(cmd: &str) -> bool {
    cmd.split_whitespace()
        .any(|word| {
            let word = unquote_token(word).to_ascii_lowercase();
            word.strip_prefix("commit.gpgsign=")
                .is_some_and(|value| FALSY.contains(&value))
        })
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
