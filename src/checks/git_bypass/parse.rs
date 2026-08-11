//! Reading a git command line: the global-option prefix, the subcommand, and the
//! tokens after it. Every check here asks this parser rather than matching the
//! literal word `git`, so a path, a wrapper or a brace group is read the same way.

use crate::checks::shell;

/// Walk the global-option prefix once, capturing the `-C` argument (if any) and
/// the subcommand token that terminates the prefix. Everything before the first
/// bare (non-`-`) token is a git global option; a `-C` after the subcommand is an
/// argument to that subcommand, not a working-directory change.
pub struct Parsed<'a> {
    pub c_path: Option<&'a str>,
    pub subcommand: Option<&'a str>,
    /// Tokens following the subcommand — the subcommand's own args/flags.
    pub args: Vec<&'a str>,
    /// A `-c`/`--config-env` was given. Config can point at a program to run
    /// (a pager, an external diff, an ssh command), so no invocation carrying one
    /// is classified read-only.
    pub sets_config: bool,
}

pub fn parse<'a>(cmd: &'a str) -> Parsed<'a> {
    let mut c_path = None;
    let mut sets_config = false;
    let mut tokens = cmd.split_whitespace();
    // Step over everything up to and including the `git` token itself, so a path
    // or a wrapper (`/usr/bin/git`, `sudo git`) parses like a bare `git`.
    for token in tokens.by_ref() {
        if token
            .rsplit('/')
            .next()
            == Some("git")
        {
            break;
        }
    }
    let mut subcommand = None;
    while let Some(tok) = tokens.next() {
        if let Some(rest) = tok.strip_prefix("-C") {
            let raw = if let Some(eq) = rest.strip_prefix('=') {
                Some(eq)
            } else if !rest.is_empty() {
                Some(rest)
            } else {
                tokens.next()
            };
            c_path = raw.map(unquote);
            continue;
        }
        // `-c key=val`, `--git-dir=...` etc. consume their own value inline or as
        // a following token; the only one we must not misread as a subcommand is
        // the two-token `-c val` form.
        if tok == "-c" || tok == "--git-dir" || tok == "--work-tree" || tok == "--namespace" {
            sets_config |= tok == "-c";
            let _ = tokens.next();
            continue;
        }
        if tok.starts_with('-') {
            sets_config |= tok.starts_with("-c") || tok.starts_with("--config-env");
            continue;
        }
        subcommand = Some(tok);
        break;
    }
    Parsed {
        c_path,
        subcommand,
        args: tokens.collect(),
        sets_config,
    }
}

pub fn git_c_path(cmd: &str) -> Option<&str> {
    parse(cmd).c_path
}

pub fn unquote(tok: &str) -> &str {
    tok.strip_prefix(['"', '\''])
        .and_then(|s| s.strip_suffix(['"', '\'']))
        .unwrap_or(tok)
}

pub fn has_token(segment: &str, flag: &str) -> bool {
    segment
        .split_whitespace()
        .any(|word| word == flag)
}

pub fn mentions_git(cmd: &str) -> bool {
    cmd.split_whitespace()
        .any(|word| word == "git" || word.ends_with("/git"))
}

/// The stage runs git, however it is reached — a path, a wrapper, a brace group.
pub fn is_git(cmd: &str) -> bool {
    shell::program(cmd) == Some("git")
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn dash_c_after_subcommand_is_not_workdir() {
        // `git branch -C old new` renames; the -C is a branch flag, no c_path.
        assert!(
            parse("git branch -C old new")
                .c_path
                .is_none()
        );
    }
}
