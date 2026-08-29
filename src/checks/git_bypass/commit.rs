//! `git commit` reading its message from stdin: the one commit shape whose file
//! set is entirely what a previous `git add` staged.

use crate::checks::git_bypass::parse::{is_git, parse};
use crate::checks::shell;

/// Flags that change neither the files that land in the commit nor the hooks that
/// run. `-a`/`--all`, `--amend`, `--allow-empty` and a pathspec are absent because
/// each of them commits something the caller did not stage by name; `--no-verify`
/// and `--no-gpg-sign` are denied outright elsewhere.
const COMMIT_FLAGS: &[&str] = &["-s", "--signoff", "-q", "--quiet"];

/// `git commit` whose message comes from stdin (`-F -`) and which names no path.
/// Every other argument has to be on `COMMIT_FLAGS`, so an unrecognized flag
/// falls through to the normal prompt rather than riding along.
pub fn is_stdin_commit(segment: &str) -> bool {
    if shell::redirects_anything(segment) {
        return false;
    }
    let stages = shell::pipeline_stages(segment);
    let Some([stage]) = stages.as_deref() else {
        return false;
    };
    if !is_git(stage) {
        return false;
    }
    let p = parse(stage);
    if p.subcommand != Some("commit") || p.sets_config || p.c_path.is_some() {
        return false;
    }
    let mut from_stdin = false;
    let mut args = p
        .args
        .iter()
        .copied();
    while let Some(arg) = args.next() {
        if COMMIT_FLAGS.contains(&arg) {
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
