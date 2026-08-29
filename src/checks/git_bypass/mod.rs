mod add;
mod commit;
mod parse;
mod read_only;
#[cfg(test)]
mod tests;

use std::path::Path;

use crate::checks::git_bypass::add::{is_blanket_add, misrooted_paths};
use crate::checks::git_bypass::parse::{git_c_path, has_token, is_git, mentions_git, parse};
use crate::checks::location::{dirs, hint, resolve, same_dir, same_repo};
use crate::checks::shell;
use crate::checks::shell::unquote_token;
use crate::input::HookInput;
use crate::output::HookOutput;

pub use crate::checks::git_bypass::add::is_explicit_add;
pub use crate::checks::git_bypass::commit::is_stdin_commit;
pub use crate::checks::git_bypass::read_only::is_read_only_segment;

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

const CD_SAME_REPO: &str = "The `cd` stays inside this repo, so it reaches no hook this command \
could not already run and the prompt it costs warns about nothing. Run the git command from \
here with the paths spelled from here, or move first with a `cd` of its own — a bare `cd` is \
auto-allowed and the working directory persists between calls.";

const BLANKET_ADD: &str = "`git add -A` / `.` / `-u` / `*` sweeps untracked scratch into the \
index — CLAUDE.md forbids it (it once staged real PII). Stage explicit paths; `git status` \
first if unsure what is untracked.";

const MISROOTED_ADD: &str = "Pathspec spelled from the repo root, not from here: ";

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
    // Last, so a bypass flag in the same command is reported before the cd. The
    // commit case first: it names the one reason a `cd` before a commit is always
    // pointless, whichever repo it lands in.
    flagged
        .or_else(|| cd_before_commit(cmd).then(|| located(CD_COMMIT, &input.cwd)))
        .or_else(|| cd_within_repo(cmd, &input.cwd))
}

/// A `cd` that stays inside the repo the shell is already in, followed by a git
/// command no allow covers. The move reaches no hook this command could not
/// already run, so the prompt Claude Code shows for it warns about nothing — and
/// the same command spelled from here takes the plain per-command approval.
fn cd_within_repo(cmd: &str, cwd: &str) -> Option<HookOutput> {
    let segments = shell::chain_segments(cmd)?;
    walk(segments, cwd, |segment, here| {
        (is_git(segment)
            && !same_dir(Path::new(here), cwd)
            && same_repo(cwd, here)
            && !is_read_only_segment(segment)
            && !is_explicit_add(segment, here))
        .then(|| located(CD_SAME_REPO, cwd))
    })
}

/// Segments in order, each judged against the directory it will actually run in:
/// a bare `cd` moves that directory for everything after it, so a path argument
/// in a later segment resolves from there and not from where the tool started.
fn walk<T>(
    segments: Vec<&str>,
    cwd: &str,
    mut judge: impl FnMut(&str, &str) -> Option<T>,
) -> Option<T> {
    let here = dirs(&segments, cwd);
    segments
        .iter()
        .zip(&here)
        .find_map(|(segment, here)| {
            let segment = segment.trim_start();
            if shell::is_bare_cd(segment) {
                return None;
            }
            judge(segment, here)
        })
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

/// True when `git -C <path>` targets the same directory the tool already runs in,
/// making the `-C` redundant.
fn points_at_cwd(cmd: &str, cwd: &str) -> bool {
    let Some(target) = git_c_path(cmd) else {
        return false;
    };
    !cwd.is_empty() && same_dir(&resolve(target, cwd), cwd)
}

/// `flags` is the part of the command git reads options from; `full` carries the
/// message too, which is where the TDD exemption looks.
fn deny(flags: &str, full: &str, cwd: &str) -> Option<HookOutput> {
    if git_c_path(flags).is_some() && points_at_cwd(flags, cwd) {
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
