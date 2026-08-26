//! Where the command will run, and how its path arguments resolve there.

use std::path::{Path, PathBuf};

use crate::checks::shell;
use crate::checks::shell::unquote_token;

/// The directory each segment runs in, in order: a bare `cd` moves it for
/// everything after it, so a path argument later in the chain resolves from
/// there and not from where the tool started. The entry for a `cd` is the
/// directory it runs *in*, the move landing on the segments behind it.
pub fn dirs(segments: &[&str], cwd: &str) -> Vec<String> {
    let mut here = cwd.to_string();
    segments
        .iter()
        .map(|segment| {
            let running_in = here.clone();
            if let Some(target) = shell::bare_cd_target(segment.trim_start()) {
                here = resolve(unquote_token(target), &here)
                    .display()
                    .to_string();
            }
            running_in
        })
        .collect()
}

/// Whether two directories sit in the same repo — a `cd` between them reaches no
/// hook the command could not already run. Neither being in a repo is not the
/// same repo: there is nothing to compare.
pub fn same_repo(a: &str, b: &str) -> bool {
    match (repo_root(a), repo_root(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Whether two paths name the same directory. Both sides are canonicalized so
/// symlinked mount paths and `.`/trailing-slash forms compare equal; if either
/// fails to resolve we fall back to a literal compare rather than guessing.
pub fn same_dir(target: &Path, cwd: &str) -> bool {
    let cwd = Path::new(cwd);
    match (target.canonicalize(), cwd.canonicalize()) {
        (Ok(t), Ok(c)) => t == c,
        _ => target == cwd,
    }
}

pub fn names_a_file(path: &str, cwd: &str) -> bool {
    if path.contains(['*', '?', '[', '$', '~']) {
        return false;
    }
    resolve(path, cwd).is_file()
}

/// A path argument as the shell will see it: relative ones hang off the directory
/// the tool runs in.
pub fn resolve(path: &str, cwd: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(cwd).join(path)
    }
}

/// The repo `cwd` sits in, kept as a literal prefix of `cwd` so a path can be
/// rebased between the two. `.git` counts as a file (worktree, submodule) or as a
/// directory holding a `HEAD` — the same walk git does, so nested repos resolve
/// the way git resolves them.
pub fn repo_root(cwd: &str) -> Option<&Path> {
    Path::new(cwd)
        .ancestors()
        .find(|dir| {
            let dot_git = dir.join(".git");
            dot_git.is_file()
                || dot_git
                    .join("HEAD")
                    .is_file()
        })
}

/// The line appended to a deny that turns on where the command runs.
pub fn hint(cwd: &str) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    match repo_root(cwd) {
        Some(root) if root != Path::new(cwd) => {
            format!("\ncwd: {cwd} — git repo root: {}", root.display())
        }
        Some(_) => format!("\ncwd: {cwd} (the git repo root)"),
        None => format!("\ncwd: {cwd} (no git repo above it)"),
    }
}

/// The same path spelled from `cwd`, given one that resolves from the repo root.
/// `root` is an ancestor of `cwd`, so the target is either under `cwd` or reached
/// by climbing out of it.
pub fn rebase_on_cwd(path: &str, cwd: &str, root: &Path) -> String {
    let target = root.join(path);
    if let Ok(under) = target.strip_prefix(cwd) {
        return under
            .display()
            .to_string();
    }
    let depth = Path::new(cwd)
        .strip_prefix(root)
        .map_or(0, |rel| {
            rel.components()
                .count()
        });
    format!("{}{path}", "../".repeat(depth))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{rebase_on_cwd, repo_root};

    #[test]
    fn rebases_under_and_out_of_cwd() {
        let root = Path::new("/r");
        assert_eq!(
            rebase_on_cwd("crate/src/x.rs", "/r/crate", root),
            "src/x.rs"
        );
        assert_eq!(rebase_on_cwd("docs/x.md", "/r/crate", root), "../docs/x.md");
        assert_eq!(
            rebase_on_cwd("docs/x.md", "/r/a/b", root),
            "../../docs/x.md"
        );
    }

    #[test]
    fn finds_the_repo_above_a_subdirectory() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert_eq!(
            repo_root(&format!("{root}/src/checks")),
            Some(Path::new(root))
        );
        assert_eq!(repo_root("/"), None);
    }
}
