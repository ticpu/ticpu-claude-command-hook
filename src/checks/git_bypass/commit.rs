//! The commit and rebase shapes an allow can carry: a commit whose file set is
//! entirely what a previous `git add` staged, and the corrections to one.
//!
//! Every shape is a bare `git <verb>`: an env prefix, a wrapper or a global option
//! can point git at another index, repo or hook set.
//!
//! A tip is amended unprompted only while no remote-tracking ref or tag reaches it,
//! as of the last fetch: past that the amend rewrites what someone else may hold.
//! A rebase is allowed only onto an ancestor of `HEAD`, where `--autosquash` folds
//! the fixups and replays nothing onto an upstream that moved; a pushed branch is
//! still rebased, the force-push after it being the prompt.
//!
//! `--fixup=amend:`, `--fixup=reword:` and `--squash` are absent: each ends in an
//! editor, at the commit or at the rebase.
//!
//! A body past `BODY_CAP` lines turns the allow into a prompt: a commit body is a
//! changelog entry, and one that long is usually narrating the diff.

use std::process::Command;

use crate::checks::shell;

/// Flags that change neither the files that land in the commit nor the hooks that
/// run. `-a`/`--all`, `--allow-empty` and a pathspec are absent because each of
/// them commits something the caller did not stage by name; `--no-verify` and
/// `--no-gpg-sign` are denied outright elsewhere.
const COMMIT_FLAGS: &[&str] = &["-s", "--signoff", "-q", "--quiet", "2>&1"];

/// Metadata only, taking a value glued with `=` or as the next word.
const COMMIT_VALUE_FLAGS: &[&str] = &["--author", "--date"];

/// What has to be true of the repo before a recognized shape is allowed.
#[derive(Debug, PartialEq)]
pub enum Needs {
    Nothing,
    UnpushedTip,
    Ancestor(String),
}

/// `git commit` whose message comes from stdin (`-F -`) and which names no path,
/// its output optionally merged and piped into consumers that write nothing.
/// Every other argument has to be on `COMMIT_FLAGS`, so an unrecognized flag
/// falls through to the normal prompt rather than riding along.
pub fn stdin_commit(segment: &str) -> Option<Needs> {
    let stage = shell::quotes_opaque(&producer(segment)?)?;
    let mut from_stdin = false;
    let mut amends = false;
    let mut args = bare_git(&stage, "commit")?.into_iter();
    while let Some(arg) = args.next() {
        if COMMIT_FLAGS.contains(&arg) {
            continue;
        }
        if arg == "--amend" {
            amends = true;
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
            return None;
        }
        from_stdin = true;
    }
    from_stdin.then_some(match amends {
        true => Needs::UnpushedTip,
        false => Needs::Nothing,
    })
}

/// A correction to a commit already made: `--amend --no-edit`, a plain `--fixup`,
/// or the `rebase --autosquash` that folds the fixups in.
pub fn correction(segment: &str) -> Option<Needs> {
    let stage = producer(segment)?;
    if let Some(args) = bare_git(&stage, "commit") {
        let args: Vec<&str> = args
            .into_iter()
            .filter(|arg| !COMMIT_FLAGS.contains(arg))
            .collect();
        return match args[..] {
            ["--amend", "--no-edit"] | ["--no-edit", "--amend"] => Some(Needs::UnpushedTip),
            ["--fixup", rev] if is_rev(rev) => Some(Needs::Nothing),
            [glued] => glued
                .strip_prefix("--fixup=")
                .filter(|rev| is_rev(rev))
                .map(|_| Needs::Nothing),
            _ => None,
        };
    }
    let mut args = bare_git(&stage, "rebase")?;
    args.retain(|arg| *arg != "2>&1" && *arg != "--keep-base");
    args.sort_unstable_by_key(|arg| !arg.starts_with('-'));
    match args[..] {
        ["--autosquash", rev] if is_rev(rev) => Some(Needs::Ancestor(rev.to_string())),
        _ => None,
    }
}

/// Whether the repo at `here` meets what a shape needs. A git that cannot be
/// asked allows nothing.
pub fn holds(needs: &Needs, here: &str) -> bool {
    let args: &[&str] = match needs {
        Needs::Nothing => return true,
        Needs::UnpushedTip => &["rev-list", "-1", "HEAD", "--not", "--remotes", "--tags"],
        Needs::Ancestor(rev) => &["merge-base", "--is-ancestor", rev, "HEAD"],
    };
    match Command::new("git")
        .args(args)
        .current_dir(here)
        .output()
    {
        Ok(out) => {
            out.status
                .success()
                && (*needs != Needs::UnpushedTip
                    || !out
                        .stdout
                        .is_empty())
        }
        Err(e) => {
            eprintln!("commit: git {} in {here}: {e}", args.join(" "));
            false
        }
    }
}

/// Past this many body lines an allowed commit is prompted instead.
pub const BODY_CAP: usize = 15;

/// Non-blank lines of a commit message under its subject, a closing paragraph of
/// trailers not counted.
pub fn body_lines(message: &str) -> usize {
    let mut paragraphs: Vec<Vec<&str>> = message
        .trim()
        .split("\n\n")
        .map(|paragraph| {
            paragraph
                .lines()
                .filter(|line| {
                    !line
                        .trim()
                        .is_empty()
                })
                .collect()
        })
        .collect();
    if paragraphs.len() > 1
        && paragraphs
            .last()
            .is_some_and(|last| {
                last.iter()
                    .all(|line| is_trailer(line))
            })
    {
        paragraphs.pop();
    }
    paragraphs
        .concat()
        .len()
        .saturating_sub(1)
}

fn is_trailer(line: &str) -> bool {
    line.split_once(": ")
        .is_some_and(|(token, _)| {
            !token.is_empty()
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// The first stage of a pipeline that writes to no path and whose consumers add
/// no side effect of their own.
fn producer(segment: &str) -> Option<String> {
    if shell::redirects_to_a_path(segment) {
        return None;
    }
    let stages = shell::pipeline_stages(segment)?;
    let (stage, consumers) = stages.split_first()?;
    consumers
        .iter()
        .all(|stage| shell::is_harmless_consumer(stage))
        .then(|| stage.to_string())
}

/// The words after `git <verb>` when the stage is exactly that.
fn bare_git<'a>(stage: &'a str, verb: &str) -> Option<Vec<&'a str>> {
    let mut words = stage.split_whitespace();
    (words.next() == Some("git") && words.next() == Some(verb)).then(|| words.collect())
}

/// A revision spelled out: no option, no `amend:` prefix, nothing a shell expands.
fn is_rev(word: &str) -> bool {
    !word.starts_with('-')
        && !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/~^@{}-".contains(c))
}

#[cfg(test)]
mod tests {
    use super::Needs::{Ancestor, Nothing, UnpushedTip};
    use super::{correction, stdin_commit};

    #[test]
    fn corrections_are_read_with_what_they_need() {
        for (cmd, needs) in [
            ("git commit --amend --no-edit", UnpushedTip),
            (
                "git commit -q --no-edit --amend 2>&1 | tail -3",
                UnpushedTip,
            ),
            ("git commit --fixup abc1234", Nothing),
            ("git commit --fixup=HEAD~2", Nothing),
            ("git rebase --autosquash master", Ancestor("master".into())),
            (
                "git rebase --keep-base --autosquash origin/master",
                Ancestor("origin/master".into()),
            ),
            ("git rebase HEAD~3 --autosquash", Ancestor("HEAD~3".into())),
        ] {
            assert_eq!(correction(cmd), Some(needs), "{cmd}");
        }
    }

    #[test]
    fn anything_else_keeps_its_prompt() {
        for cmd in [
            "git commit --amend",
            "git commit --amend --no-edit -a",
            "git commit --amend --no-edit src/main.rs",
            "git commit --fixup=amend:abc1234",
            "git commit --fixup=reword:abc1234",
            "git commit --squash abc1234",
            "git commit --fixup abc1234 -m 'x'",
            "git commit --fixup \"$REV\"",
            "git rebase --autosquash",
            "git rebase -i --autosquash master",
            "git rebase --autosquash --exec 'make' master",
            "git rebase --autosquash --onto x master",
            "git rebase --autosquash master topic",
            "GIT_SEQUENCE_EDITOR=x git rebase --autosquash master",
            "git -c core.editor=x commit --amend --no-edit",
            "sudo git commit --fixup abc1234",
            "git commit --fixup abc1234 > /zztest/log",
        ] {
            assert_eq!(correction(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn a_body_is_counted_without_its_subject_and_trailers() {
        let message = "feat: x\n\nwhy\nwhat\n\nmore\n\nCo-Authored-By: A <a@b.c>\nRefs: #1";
        assert_eq!(super::body_lines(message), 3);
        assert_eq!(super::body_lines("feat: x"), 0);
    }

    #[test]
    fn an_amend_on_the_stdin_shape_needs_an_unpushed_tip() {
        assert_eq!(stdin_commit("git commit -F -"), Some(Nothing));
        assert_eq!(stdin_commit("git commit --amend -F -"), Some(UnpushedTip));
        assert_eq!(stdin_commit("git commit --amend"), None);
    }
}
