use crate::output::HookOutput;

const REASON: &str =
    "Broad `find` walk blocked (/, ~, $HOME, the home dir, or the GIT repo parent). \
These trees are massive and slow to walk. Scope to a specific project dir or subdir, use \
`rg --files <dir>`, or hand a narrow path to the Explore agent. A find inside one repo \
(e.g. ~/GIT/eido/...) is fine — its parent is not.";

pub fn check(command: &str) -> Option<HookOutput> {
    has_broad_find(command).then(|| HookOutput::deny("PreToolUse", REASON))
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

fn is_broad_target(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    let t = path
        .strip_suffix('/')
        .unwrap_or(path);
    if matches!(t, "~" | "$HOME") {
        return true;
    }
    // The GIT repo parent itself (real or ~ form); a specific repo under it is fine.
    if t.contains('/')
        && t.rsplit('/')
            .next()
            == Some("GIT")
    {
        return true;
    }
    // The bare home directory: /home/<user>, but not a subdir of it.
    let comps: Vec<&str> = t
        .split('/')
        .collect();
    comps.len() == 3 && comps[0].is_empty() && comps[1] == "home" && !comps[2].is_empty()
}

#[cfg(test)]
mod tests {
    use super::has_broad_find;

    #[test]
    fn blocks_broad_roots() {
        for cmd in [
            "find / -name foo",
            "find ~",
            "find ~/",
            "find ~/GIT",
            "find ~/GIT/",
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
}
