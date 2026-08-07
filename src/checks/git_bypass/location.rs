//! Where the command will run, and how its path arguments resolve there.

use std::path::{Path, PathBuf};

use crate::checks::git_bypass::parse::git_c_path;

/// True when `git -C <path>` targets the same directory the tool already runs in,
/// making the `-C` redundant. Both sides are canonicalized so symlinked mount
/// paths and `.`/trailing-slash forms compare equal; if either fails to resolve
/// we fall back to a literal compare rather than guessing.
pub fn points_at_cwd(cmd: &str, cwd: &str) -> bool {
    let Some(target) = git_c_path(cmd) else {
        return false;
    };
    if cwd.is_empty() {
        return false;
    }
    let target = resolve(target, cwd);
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
