//! `git add`: which staging is a named-file operation and which is a sweep.

use std::path::Path;

use crate::checks::git_bypass::parse::parse;
use crate::checks::git_bypass::read_only::git_producer;
use crate::checks::location::{names_a_file, rebase_on_cwd, repo_root, resolve};
use crate::checks::shell::unquote_token;

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

pub fn is_explicit_add(segment: &str, cwd: &str) -> bool {
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
        let path = unquote_token(arg);
        if BLANKET_PATHS.contains(&path) || !names_a_file(path, cwd) {
            return false;
        }
        paths += 1;
    }
    paths > 0
}

/// Pathspecs spelled from the repo root instead of from the directory the command
/// runs in, each paired with the spelling that would work. Only a path that
/// resolves from the root qualifies: one that resolves nowhere is a deletion being
/// staged, or a typo this cannot correct.
pub fn misrooted_paths(cmd: &str, cwd: &str) -> Vec<(String, String)> {
    let p = parse(cmd);
    if p.subcommand != Some("add") || cwd.is_empty() {
        return Vec::new();
    }
    let Some(root) = repo_root(cwd).filter(|root| *root != Path::new(cwd)) else {
        return Vec::new();
    };
    p.args
        .iter()
        .map(|arg| unquote_token(arg))
        .filter(|arg| {
            !arg.starts_with('-')
                && !arg.contains(['*', '?', '[', '$', '~'])
                && !BLANKET_PATHS.contains(arg)
                && !resolve(arg, cwd).exists()
                && root
                    .join(arg)
                    .exists()
        })
        .map(|arg| (arg.to_string(), rebase_on_cwd(arg, cwd, root)))
        .collect()
}

/// `-A`/`--all`/`-u`/`--update` — including inside a short bundle like `-Av` — or a
/// pathspec standing for the whole tree. Quotes are stripped per token, since
/// `git add "."` reaches git as the same pathspec the bare form does.
pub fn is_blanket_add(cmd: &str) -> bool {
    let p = parse(cmd);
    if p.subcommand != Some("add") {
        return false;
    }
    p.args
        .iter()
        .map(|arg| unquote_token(arg))
        .any(|arg| {
            BLANKET_PATHS.contains(&arg)
                || matches!(arg, "--all" | "--update")
                || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains(['A', 'u']))
        })
}
