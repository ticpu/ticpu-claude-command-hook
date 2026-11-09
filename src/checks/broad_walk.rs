//! Denies a `find` walk of `/`, the home directory or the GIT repo parent; one
//! scoped to a repo under it passes. A trailing glob is judged on its parent,
//! `~/GIT/*` being the same walk under another spelling.
//!
//! An `ls` or `tree` of the GIT parent or the home directory is denied for a
//! different reason: they hold hundreds of entries and a repo path is built from
//! its name, so the listing is browsing to guess. `/` is off that half, printing
//! two dozen names that answer a real question, and so is `ls -d`, which names
//! the directory instead of listing it; `tree -d` still walks.
//!
//! A path is judged where it lands, not as spelled: `..`, `.` and a lister with
//! no operand resolve from the directory the segment runs in, a `cd` earlier in
//! the chain included, so `cd ~/GIT/x && ls ..` is the GIT parent. A `~` or `$`
//! path is judged as spelled, the hook not expanding either.
//!
//! A lister is a neutral segment to every allow, so this denies ahead of them.

use std::path::PathBuf;

use path_clean::PathClean;

use crate::checks::location;
use crate::checks::shell;
use crate::output::HookOutput;

const FIND_REASON: &str = "Broad `find` walk blocked (/, ~, $HOME, the home dir, or the GIT repo parent). \
These trees are massive and slow to walk. Scope to a specific project dir or subdir, use \
`rg --files <dir>`, or hand a narrow path to the Explore agent. A find inside one repo \
(e.g. ~/GIT/eido/...) is fine — its parent is not.";

const LIST_REASON: &str = "Listing the GIT repo parent or the home dir blocked. ~/GIT holds hundreds of \
folders, so a listing is browsing to guess at a name rather than reading an answer. Repo paths \
are ~/GIT/<repository-name> verbatim — build the path from the repo name and use it, or ask \
which repo. Listing one repo (e.g. ~/GIT/eido) is fine, and so is `ls -d`.";

#[derive(Debug, PartialEq)]
enum Walk {
    Find,
    List,
}

pub fn check(command: &str, cwd: &str) -> Option<HookOutput> {
    match broad_walk(command, cwd)? {
        Walk::Find => Some(HookOutput::deny("PreToolUse", FIND_REASON)),
        Walk::List => Some(HookOutput::deny("PreToolUse", LIST_REASON)),
    }
}

/// The broadest walk in the command, a `find` outranking a listing.
fn broad_walk(command: &str, cwd: &str) -> Option<Walk> {
    let segments = shell::chain_segments(command)?;
    let dirs = location::dirs(&segments, cwd);
    let walks: Vec<Walk> = segments
        .iter()
        .zip(&dirs)
        .flat_map(|(segment, dir)| {
            shell::pipeline_stages(segment)
                .into_iter()
                .flatten()
                .filter_map(move |stage| stage_walk(stage, dir))
        })
        .collect();
    if walks.contains(&Walk::Find) {
        return Some(Walk::Find);
    }
    walks
        .into_iter()
        .next()
}

fn stage_walk(stage: &str, dir: &str) -> Option<Walk> {
    let word = shell::program(stage)?;
    let args = shell::program_args(stage)?;
    if word == "find" {
        return find_paths(&args)
            .into_iter()
            .any(|p| is_broad_target(&landing(p, dir)))
            .then_some(Walk::Find);
    }
    if !matches!(word, "ls" | "ll" | "tree") {
        return None;
    }
    // `ls -d` names the directory instead of listing it, which is not a browse.
    // `tree -d` still walks the tree, so it does not qualify.
    if word != "tree"
        && args
            .iter()
            .any(|a| {
                *a == "--directory"
                    || (a.starts_with('-') && !a.starts_with("--") && a.contains('d'))
            })
    {
        return None;
    }
    let value_options: &[&str] = if word == "tree" {
        &[
            "-L",
            "-P",
            "-I",
            "-o",
            "-H",
            "-T",
            "--filelimit",
            "--timefmt",
            "--compress",
        ]
    } else {
        &[
            "-I",
            "-T",
            "-w",
            "--hide",
            "--ignore",
            "--block-size",
            "--format",
            "--sort",
        ]
    };
    let mut operands = Vec::new();
    let mut tokens = args.into_iter();
    while let Some(arg) = tokens.next() {
        if value_options.contains(&arg) {
            tokens.next();
        } else if !arg.starts_with('-') {
            operands.push(arg);
        }
    }
    if operands.is_empty() {
        operands.push(".");
    }
    operands
        .into_iter()
        .any(|p| is_browse_target(&landing(p, dir)))
        .then_some(Walk::List)
}

/// find's starting points: the operands between its leading options and the
/// first expression token, `.` when there are none.
fn find_paths<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        match *arg {
            "-H" | "-L" | "-P" => i += 1,
            "-D" => i += 2,
            a if a.starts_with("-O") => i += 1,
            _ => break,
        }
    }
    let paths: Vec<&str> = args[i.min(args.len())..]
        .iter()
        .take_while(|a| !a.starts_with(['-', '(', '!']))
        .copied()
        .collect();
    if paths.is_empty() { vec!["."] } else { paths }
}

/// Where an operand lands once the shell has resolved it from `dir`.
fn landing(operand: &str, dir: &str) -> String {
    let spelled = normalize(shell::unquote_token(operand));
    if spelled.starts_with(['~', '$']) {
        return spelled.to_string();
    }
    let landed: PathBuf = location::resolve(spelled, dir).clean();
    landed
        .display()
        .to_string()
}

/// The bare home dir, in any spelling.
fn is_home(t: &str) -> bool {
    if matches!(t, "~" | "$HOME") {
        return true;
    }
    let comps: Vec<&str> = t
        .split('/')
        .collect();
    comps.len() == 3 && comps[0].is_empty() && comps[1] == "home" && !comps[2].is_empty()
}

/// The GIT repo parent itself (real or `~` form); a specific repo under it is fine.
fn is_git_parent(t: &str) -> bool {
    t.contains('/')
        && t.rsplit('/')
            .next()
            == Some("GIT")
}

fn is_browse_target(path: &str) -> bool {
    let t = normalize(path);
    is_home(t) || is_git_parent(t)
}

fn is_broad_target(path: &str) -> bool {
    let t = normalize(path);
    t == "/" || is_home(t) || is_git_parent(t)
}

/// A trailing glob stands for the entries of its parent, so `~/GIT/*` is a walk of
/// `~/GIT` under another spelling.
fn normalize(path: &str) -> &str {
    if path == "/" {
        return path;
    }
    let t = path
        .strip_suffix('/')
        .unwrap_or(path);
    let last = t
        .rsplit('/')
        .next()
        .unwrap_or(t);
    if !last.contains(['*', '?']) {
        return t;
    }
    match &t[..t.len() - last.len()] {
        "" => ".",
        "/" => "/",
        parent => parent
            .strip_suffix('/')
            .unwrap_or(parent),
    }
}

#[cfg(test)]
mod tests {
    use super::{Walk, broad_walk};

    const REPO: &str = "/home/u/GIT/eido";

    fn walk(cmd: &str) -> Option<Walk> {
        broad_walk(cmd, "")
    }

    #[test]
    fn blocks_broad_roots() {
        for cmd in [
            "find / -name foo",
            "find /*",
            "find ~",
            "find ~/",
            "find ~/GIT",
            "find ~/GIT/",
            "find ~/GIT/*",
            "find $HOME",
            "find $HOME/",
            "find /home/jerome.poulin",
            "find /home/jerome.poulin/GIT",
            "find /mnt/bcachefs/@home/jerome.poulin/GIT -type f",
            "find -L ~/GIT -name x",
            "find src ~/GIT -name x",
            "rg --files | find / -name y",
            "/usr/bin/find / -name z",
        ] {
            assert_eq!(walk(cmd), Some(Walk::Find), "should block: {cmd}");
        }
    }

    #[test]
    fn allows_scoped_paths() {
        for cmd in [
            "find ~/GIT/eido -name x",
            "find /home/jerome.poulin/GIT/NENA911",
            "find /mnt/bcachefs/@home/jerome.poulin/GIT/eido -type f",
            "find ~/GIT/eido/*",
            "find . -name foo",
            "find src -type f",
            "find /home/jerome.poulin/projects",
            "find $HOME/projects",
            "git log --find-renames",
            "echo find more stuff",
        ] {
            assert_eq!(walk(cmd), None, "should allow: {cmd}");
        }
    }

    #[test]
    fn blocks_browsing_the_repo_parent() {
        for cmd in [
            "ls ~/GIT",
            "ls ~/GIT/",
            "ls -l /home/jerome.poulin/GIT",
            "ls /home/jerome.poulin/GIT/ | head -20",
            "ls ~/GIT/*",
            "tree -L 1 ~/GIT",
            "ls ~",
            "ls $HOME",
            "ls /home/jerome.poulin",
            "cd /tmp && ls ~/GIT",
            "ls src; ls ~/GIT",
        ] {
            assert_eq!(walk(cmd), Some(Walk::List), "should block: {cmd}");
        }
    }

    #[test]
    fn allows_scoped_listings() {
        for cmd in [
            "ls ~/GIT/eido",
            "ls -l /home/jerome.poulin/GIT/NENA911/src",
            "ls -d ~/GIT",
            "ls -ld /home/jerome.poulin",
            "ls",
            "ls /usr/src",
            "ls ~/GIT/eido/*",
            "echo ls ~/GIT",
            "find ~/GIT/eido -name x",
        ] {
            assert_eq!(walk(cmd), None, "should allow: {cmd}");
        }
    }

    #[test]
    fn judges_a_relative_path_where_it_lands() {
        for (cmd, cwd, expected) in [
            (
                "cd /home/u/GIT/odf && test -f docs/x.md; echo r=$?; ls docs scripts ..",
                "/srv",
                Some(Walk::List),
            ),
            ("ls ..", REPO, Some(Walk::List)),
            ("ls ../", REPO, Some(Walk::List)),
            ("ls ../*", REPO, Some(Walk::List)),
            ("ls src/../..", REPO, Some(Walk::List)),
            ("ls ../..", REPO, Some(Walk::List)),
            ("ls", "/home/u/GIT", Some(Walk::List)),
            ("cd .. && ls", REPO, Some(Walk::List)),
            ("cd .. && tree -L 1", REPO, Some(Walk::List)),
            ("find .. -name x", REPO, Some(Walk::Find)),
            ("cd / && find . -name x", REPO, Some(Walk::Find)),
            ("find -name x", "/home/u/GIT", Some(Walk::Find)),
            ("ls ../eido-other", REPO, None),
            ("cd .. && ls eido", REPO, None),
            ("ls -d ..", REPO, None),
            ("ls", REPO, None),
            ("find . -name x", REPO, None),
            ("cd /home/u/GIT/odf && ls docs", "/srv", None),
        ] {
            assert_eq!(broad_walk(cmd, cwd), expected, "{cmd} from {cwd}");
        }
    }
}
