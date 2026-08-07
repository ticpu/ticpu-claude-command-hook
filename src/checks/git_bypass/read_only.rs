//! Which git invocations provably only read. Fail-safe throughout: anything not
//! recognized falls through to the normal permission prompt.

use crate::checks::git_bypass::parse::{is_git, parse, unquote};
use crate::checks::shell;

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

pub fn is_read_only_segment(segment: &str) -> bool {
    git_producer(segment).is_some_and(is_read_only)
}

/// The git invocation a segment produces its output with — but only when the
/// segment can be judged by that invocation alone. A stdout redirect or a consumer
/// that is not display-only (`| sh`, `| xargs rm`) would otherwise ride along on
/// whatever the invocation is allowed.
pub fn git_producer(segment: &str) -> Option<&str> {
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

/// Fail-safe classifier: returns true only when the command is *provably*
/// read-only. Subcommands in `ALWAYS_READ_ONLY` qualify unconditionally; the
/// mode-dependent ones (branch/tag/config/remote/reflog/symbolic-ref) qualify
/// only in explicitly-whitelisted read-only forms. Everything else — including
/// any unrecognized flag or subcommand — returns false and falls through to the
/// normal permission prompt, so an unforeseen write mode is never auto-allowed.
pub fn is_read_only(cmd: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::is_read_only;

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
