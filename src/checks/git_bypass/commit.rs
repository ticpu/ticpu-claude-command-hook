//! `git commit` reading its message from stdin: the one commit shape whose file
//! set is entirely what a previous `git add` staged.

use crate::checks::shell;

/// Flags that change neither the files that land in the commit nor the hooks that
/// run. `-a`/`--all`, `--amend`, `--allow-empty` and a pathspec are absent because
/// each of them commits something the caller did not stage by name; `--no-verify`
/// and `--no-gpg-sign` are denied outright elsewhere.
const COMMIT_FLAGS: &[&str] = &["-s", "--signoff", "-q", "--quiet", "2>&1"];

/// Metadata only, taking a value glued with `=` or as the next word.
const COMMIT_VALUE_FLAGS: &[&str] = &["--author", "--date"];

/// `git commit` whose message comes from stdin (`-F -`) and which names no path,
/// its output optionally merged and piped into consumers that write nothing.
/// Every other argument has to be on `COMMIT_FLAGS`, so an unrecognized flag
/// falls through to the normal prompt rather than riding along.
pub fn is_stdin_commit(segment: &str) -> bool {
    if shell::redirects_to_a_path(segment) {
        return false;
    }
    let Some(stages) = shell::pipeline_stages(segment) else {
        return false;
    };
    let (stage, consumers) = stages
        .split_first()
        .expect("pipeline_stages never yields an empty list");
    if !consumers
        .iter()
        .all(|stage| shell::is_harmless_consumer(stage))
    {
        return false;
    }
    let Some(stage) = quotes_opaque(stage) else {
        return false;
    };
    let Some(args) = bare_git(&stage, "commit") else {
        return false;
    };
    let mut from_stdin = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if COMMIT_FLAGS.contains(&arg) {
            continue;
        }
        if COMMIT_VALUE_FLAGS.contains(&arg) {
            args.next();
            continue;
        }
        if COMMIT_VALUE_FLAGS
            .iter()
            .any(|flag| {
                arg.strip_prefix(flag)
                    .is_some_and(|rest| rest.starts_with('='))
            })
        {
            continue;
        }
        let value = match arg {
            "-F" | "--file" => args.next(),
            "--file=-" => Some("-"),
            _ => arg
                .strip_prefix("-F")
                .filter(|glued| !glued.is_empty()),
        };
        if value != Some("-") || from_stdin {
            return false;
        }
        from_stdin = true;
    }
    from_stdin
}

/// The words after `git <verb>` when the stage is exactly that: an env prefix, a
/// wrapper or a global option can point git at another index, repo or hook set.
fn bare_git<'a>(stage: &'a str, verb: &str) -> Option<Vec<&'a str>> {
    let mut words = stage.split_whitespace();
    (words.next() == Some("git") && words.next() == Some(verb)).then(|| words.collect())
}

/// Each quoted span as one opaque word, so `--author="A B"` stays one argument and
/// a quoted pathspec still reads as an argument nothing here accepts.
fn quotes_opaque(stage: &str) -> Option<String> {
    let mut out = stage.to_owned();
    for span in shell::quoted_spans(stage)?
        .into_iter()
        .rev()
    {
        out.replace_range(span, "Q");
    }
    Some(out)
}
