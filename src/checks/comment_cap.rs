//! A run of whole-line comments is at most two lines: one needing a third is a name
//! or an extraction the code is missing, or a reason that belongs in the commit.
//!
//! Out of scope, each for its own reason: `///` and `//!`, which are API
//! documentation; the comment block that opens a file, which is where a file's
//! notes go in a language without doc comments; and a block carrying
//! `comment-cap-exempt: <reason>`, for the site that really needs more.
//!
//! Only a block the edit adds or changes is judged — one already on disk is left
//! to the edit that touches it, so old code is not a wall of refusals. Block
//! comments and docstrings are not read.
//!
//! It fires in every repo by default. A foreign checkout keeps its maintainer's
//! style through the `comment-ignore <repo-name>` verb, shown to the user on the
//! first refusal per session and repo.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::checks::edited;
use crate::checks::location::repo_root;
use crate::checks::marker;
use crate::config;
use crate::input::HookInput;
use crate::output::HookOutput;

const CAP: usize = 2;
const EXEMPT: &str = "comment-cap-exempt:";

const SLASHES: &[&str] = &[
    "rs", "c", "h", "cc", "cpp", "hpp", "go", "js", "jsx", "ts", "tsx", "java", "kt", "swift",
    "zig",
];
const HASHES: &[&str] = &[
    "sh", "bash", "zsh", "py", "toml", "yaml", "yml", "pp", "rb", "pl", "mk",
];

pub fn pre_tool_use(input: &HookInput) -> Option<HookOutput> {
    let path = input.file_path();
    let leader = leader(path)?;
    let repo = repo_name(path);
    if let Some(repo) = &repo {
        match config::load() {
            Ok(config) => {
                if config
                    .comment_cap_ignore
                    .contains(repo)
                {
                    return None;
                }
            }
            Err(e) => eprintln!("comment_cap: {e:#}"),
        }
    }
    let before = edited::before(input);
    let after = edited::after(input, &before);
    let known: HashSet<String> = blocks(&before, leader)
        .into_iter()
        .map(|block| block.text)
        .collect();
    let long = blocks(&after, leader)
        .into_iter()
        .find(|block| !known.contains(&block.text))?;
    let mut refused = HookOutput::deny(
        "PreToolUse",
        &format!(
            "A comment is at most {CAP} lines, and this edit leaves one of {} at line {}:\n\n{}\n\n\
             Cut it to the one clause naming what the code cannot say — the trap, the invariant, \
             why this branch exists — or extract a named function or type. What was wrong and why \
             this fix goes in the commit body; a file's own notes go in the comment block that \
             opens it. A site that really needs more carries `{EXEMPT} <reason>` in the block.",
            block_len(&long.text),
            long.line,
            long.text
        ),
    );
    refused.system_message = repo.and_then(|repo| opt_out(&input.session_id, &repo));
    Some(refused)
}

fn block_len(text: &str) -> usize {
    text.lines()
        .count()
}

/// The command that turns the cap off for this repo, once per session and repo.
fn opt_out(session: &str, repo: &str) -> Option<String> {
    let shown = format!("comment-ignore-shown-{session}-{repo}");
    if marker::present(&shown) {
        return None;
    }
    if let Some(dir) = marker::dir() {
        if let Err(e) = fs::create_dir_all(&dir).and_then(|()| fs::write(dir.join(&shown), "")) {
            eprintln!("comment_cap: recording {shown} failed: {e} — the hint will repeat");
        }
    }
    Some(format!(
        "Comment cap refused an edit in {repo}. To turn it off for that repo:\n  {} \
         comment-ignore {repo}",
        env!("CARGO_PKG_NAME")
    ))
}

fn repo_name(path: &str) -> Option<String> {
    let dir = Path::new(path)
        .parent()?
        .to_str()?;
    repo_root(dir)?
        .file_name()?
        .to_str()
        .map(str::to_string)
}

/// The line-comment leader of the file's language, `None` for one this does not read.
fn leader(path: &str) -> Option<&'static str> {
    let path = Path::new(path);
    if path
        .file_name()
        .is_some_and(|name| name == "Makefile")
    {
        return Some("#");
    }
    let extension = path
        .extension()?
        .to_str()?;
    if SLASHES.contains(&extension) {
        Some("//")
    } else if HASHES.contains(&extension) {
        Some("#")
    } else {
        None
    }
}

struct Block {
    /// 1-indexed line the block starts on.
    line: usize,
    /// Its lines, trimmed, so re-indenting a block does not make it a new one.
    text: String,
}

/// Runs of whole-line comments past the cap, the file's opening block and exempt
/// ones left out.
fn blocks(text: &str, leader: &str) -> Vec<Block> {
    let mut found = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    let mut code_seen = false;
    let mut opening = false;
    let lines: Vec<&str> = text
        .lines()
        .collect();
    for (i, line) in lines
        .iter()
        .copied()
        .chain([""])
        .enumerate()
    {
        let trimmed = line.trim();
        if is_comment(trimmed, leader) {
            if run.is_empty() {
                opening = !code_seen;
            }
            run.push(trimmed);
            continue;
        }
        if run.len() > CAP
            && !opening
            && !run
                .iter()
                .any(|line| line.contains(EXEMPT))
        {
            found.push(Block {
                line: i + 1 - run.len(),
                text: run.join("\n"),
            });
        }
        run.clear();
        code_seen |= !trimmed.is_empty() && !trimmed.starts_with("#!");
    }
    found
}

fn is_comment(trimmed: &str, leader: &str) -> bool {
    trimmed.starts_with(leader)
        && match leader {
            "//" => !trimmed.starts_with("///") && !trimmed.starts_with("//!"),
            _ => !trimmed.starts_with("#!"),
        }
}

#[cfg(test)]
mod tests {
    use super::{blocks, leader};

    fn starts(text: &str, leader: &str) -> Vec<usize> {
        blocks(text, leader)
            .iter()
            .map(|block| block.line)
            .collect()
    }

    #[test]
    fn a_run_past_two_lines_is_found_where_it_starts() {
        let text = "fn a() {}\n\n// one\n// two\nfn b() {}\n    // one\n    // two\n    // three\nfn c() {}\n";
        assert_eq!(starts(text, "//"), vec![6]);
        let text = "x = 1\n# one\n# two\n# three\ny = 2\n";
        assert_eq!(starts(text, "#"), vec![2]);
    }

    #[test]
    fn docs_the_opening_block_and_exempt_blocks_are_left_alone() {
        let text = "// notes\n// for\n// the file\n// itself\n\nfn a() {}\n/// one\n/// two\n/// three\nfn b() {}\n// comment-cap-exempt: a table\n// a\n// b\n// c\nfn c() {}\n";
        assert_eq!(starts(text, "//"), Vec::<usize>::new());
        let text = "#!/bin/sh\n# notes\n# for\n# the file\nset -e\n";
        assert_eq!(starts(text, "#"), Vec::<usize>::new());
    }

    #[test]
    fn a_block_at_the_end_of_the_file_counts() {
        assert_eq!(starts("x()\n// a\n// b\n// c", "//"), vec![2]);
    }

    #[test]
    fn the_leader_follows_the_language() {
        assert_eq!(leader("/x/src/main.rs"), Some("//"));
        assert_eq!(leader("/x/run.sh"), Some("#"));
        assert_eq!(leader("/x/Makefile"), Some("#"));
        assert_eq!(leader("/x/README.md"), None);
        assert_eq!(leader("/x/noext"), None);
    }
}
