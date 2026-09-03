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

pub fn check(command: &str) -> Option<HookOutput> {
    if has_broad_find(command) {
        return Some(HookOutput::deny("PreToolUse", FIND_REASON));
    }
    lists_a_broad_dir(command).then(|| HookOutput::deny("PreToolUse", LIST_REASON))
}

fn is_find_token(tok: &str) -> bool {
    tok == "find" || tok.ends_with("/find")
}

fn has_broad_find(command: &str) -> bool {
    let tokens: Vec<&str> = command
        .split_whitespace()
        .collect();
    for (i, tok) in tokens
        .iter()
        .enumerate()
    {
        if !is_find_token(tok) {
            continue;
        }
        // The first non-option token after `find` is its leading path argument.
        let path = tokens[i + 1..]
            .iter()
            .find(|t| !t.starts_with('-'));
        if let Some(p) = path {
            if is_broad_target(p) {
                return true;
            }
        }
    }
    false
}

fn is_lister(word: &str) -> bool {
    matches!(word, "ls" | "ll" | "tree")
}

/// A directory lister pointed at the GIT parent or the home dir. `/` is not on the
/// list: it prints a couple of dozen names and answers a real question, where the
/// two below only hand back a guess.
fn lists_a_broad_dir(command: &str) -> bool {
    let Some(segments) = shell::chain_segments(command) else {
        return false;
    };
    segments
        .iter()
        .filter_map(|seg| shell::pipeline_stages(seg))
        .flatten()
        .any(|stage| {
            let Some(word) = shell::program(stage) else {
                return false;
            };
            if !is_lister(word) {
                return false;
            }
            let Some(args) = shell::program_args(stage) else {
                return false;
            };
            // `ls -d` names the directory instead of listing it, which is not a
            // browse. `tree -d` still walks the tree, so it does not qualify.
            if word != "tree"
                && args
                    .iter()
                    .any(|a| {
                        *a == "--directory" || (a.starts_with('-') && !a.starts_with("--") && a.contains('d'))
                    })
            {
                return false;
            }
            args.iter()
                .filter(|a| !a.starts_with('-'))
                .any(|a| is_browse_target(shell::unquote_token(a)))
        })
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
    if last.contains('*') || last.contains('?') {
        return t
            .strip_suffix(last)
            .and_then(|p| p.strip_suffix('/'))
            .unwrap_or(t);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::{has_broad_find, lists_a_broad_dir};

    #[test]
    fn blocks_broad_roots() {
        for cmd in [
            "find / -name foo",
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
            "rg --files | find / -name y",
            "/usr/bin/find / -name z",
        ] {
            assert!(has_broad_find(cmd), "should block: {cmd}");
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
            assert!(!has_broad_find(cmd), "should allow: {cmd}");
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
            assert!(lists_a_broad_dir(cmd), "should block: {cmd}");
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
            assert!(!lists_a_broad_dir(cmd), "should allow: {cmd}");
        }
    }
}
