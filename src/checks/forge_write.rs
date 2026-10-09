//! Nothing is published under the user's name unprompted: creating, editing or
//! commenting on a PR, MR or issue through `gh` or `glab` is forced to a prompt,
//! which a hook `ask` gets in every permission mode, auto mode included.
//!
//! The group-token wrappers (`glab-cauca`, `glab-ng911`) are exempt by being other
//! programs: they post as a bot, so it is not the user's name that is spent.
//!
//! A `gh` create must also take its body from a `pr-body*.md` / `issue-body*.md`
//! file, re-read here when the command runs, so what the user read is what posts
//! and a file written around the Edit/Write gate is still checked. For the same
//! reason nothing else may run beside it: a segment in front could rewrite the
//! file after this read. `glab` has no body-file flag and gets the prompt alone.
//!
//! It is called twice from `dispatch`: the refusals ahead of every prompt another
//! check can raise, the prompt after every other refusal.

use std::fs;

use crate::checks::location::{dirs, resolve};
use crate::checks::pr_body::{self, Kind};
use crate::checks::shell;
use crate::checks::vouch;
use crate::input::HookInput;
use crate::output::HookOutput;

const FORM: &str = "A public PR or issue is opened from a file the user has read. Write the body \
to scratch/pr-body-<topic>.md (scratch/issue-body-<topic>.md for an issue), give them the title, \
and on their go run it alone:
  gh pr create --title '…' --body-file scratch/pr-body-<topic>.md
Not accepted: --body, --fill, --editor, --web, --template, --recover, stdin, a second body flag, \
a quoted or differently named file, a pipe into gh, or anything chained beside it other than a \
`cd`, an `echo` or a `git add`.";

const BODY_FLAGS: &[&str] = &[
    "--body",
    "--fill",
    "--fill-first",
    "--fill-verbose",
    "--editor",
    "--web",
    "--recover",
    "--template",
];

/// Short options of `gh pr|issue create` that supply or replace the body.
const BODY_SHORTS: &str = "bfewTF";

pub fn deny(input: &HookInput) -> Option<HookOutput> {
    judge(input).filter(is_deny)
}

pub fn ask(input: &HookInput) -> Option<HookOutput> {
    judge(input).filter(|decision| !is_deny(decision))
}

fn is_deny(decision: &HookOutput) -> bool {
    decision
        .hook_specific_output
        .as_ref()
        .and_then(|specific| {
            specific
                .permission_decision
                .as_deref()
        })
        == Some("deny")
}

struct Write<'a> {
    program: &'a str,
    group: &'a str,
    verb: &'a str,
}

impl Write<'_> {
    fn is_gh_create(&self) -> bool {
        self.program == "gh" && matches!(self.verb, "create" | "new")
    }

    fn prompt(&self, body: &str) -> HookOutput {
        HookOutput::ask(
            "PreToolUse",
            &format!(
                "Publishes under your name: {} {} {}{body}.",
                self.program, self.group, self.verb
            ),
        )
    }
}

fn judge(input: &HookInput) -> Option<HookOutput> {
    let cmd = input.command();
    let Some(segments) = shell::chain_segments(cmd) else {
        return unsplit(cmd);
    };
    let here = dirs(&segments, &input.cwd);
    for (i, segment) in segments
        .iter()
        .enumerate()
    {
        let Some(stages) = shell::pipeline_stages(segment) else {
            return unsplit(cmd);
        };
        for (j, stage) in stages
            .iter()
            .enumerate()
        {
            let Some(write) = forge_write(stage) else {
                continue;
            };
            if !write.is_gh_create() {
                return Some(write.prompt(""));
            }
            let alone = j == 0
                && segments
                    .iter()
                    .zip(&here)
                    .enumerate()
                    .all(|(k, (other, dir))| {
                        k == i
                            || (!shell::redirects_anything(other)
                                && vouch::is_harmless_segment(other, dir))
                    });
            if !alone {
                return Some(HookOutput::deny("PreToolUse", FORM));
            }
            return Some(create(&write, stage, &here[i]));
        }
    }
    None
}

/// A command the splitter gave up on — a heredoc, an unbalanced quote. The words
/// in front of the marker still name the program and its verb.
fn unsplit(cmd: &str) -> Option<HookOutput> {
    let tokens: Vec<&str> = shell::before_heredoc(cmd)
        .split_whitespace()
        .collect();
    tokens
        .iter()
        .enumerate()
        .find_map(|(i, token)| {
            let program = forge(shell::unquote_token(token))?;
            let args: Vec<&str> = tokens[i + 1..]
                .iter()
                .copied()
                .take_while(|word| !word.ends_with([';', '&', '|']))
                .collect();
            let (group, verb) = verb_pair(program, &args)?;
            Some(Write {
                program,
                group,
                verb,
            })
        })
        .map(|write| match write.is_gh_create() {
            true => HookOutput::deny("PreToolUse", FORM),
            false => write.prompt(""),
        })
}

fn create(write: &Write, stage: &str, here: &str) -> HookOutput {
    let form = || HookOutput::deny("PreToolUse", FORM);
    let Some(opaque) = shell::quotes_opaque(stage) else {
        return form();
    };
    let Some(path) = shell::program_args(&opaque)
        .as_deref()
        .and_then(body_file)
    else {
        return form();
    };
    let wanted = match write.group {
        "pr" => Kind::Pr,
        _ => Kind::Issue,
    };
    if pr_body::kind(path) != Some(wanted) {
        return form();
    }
    match fs::read_to_string(resolve(path, here)) {
        Ok(text) => {
            let faults = pr_body::faults(&text, wanted);
            match faults.is_empty() {
                true => write.prompt(&format!(
                    ", body from {path} ({} lines)",
                    text.lines()
                        .count()
                )),
                false => HookOutput::deny("PreToolUse", &pr_body::refusal(&faults)),
            }
        }
        Err(e) => write.prompt(&format!(
            ", body from {path} — which could not be read ({e}), so nothing checked it"
        )),
    }
}

/// The one file a create takes its body from, `None` when the body comes from
/// anywhere else or from two places.
fn body_file<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut file = None;
    let mut args = args
        .iter()
        .copied();
    while let Some(arg) = args.next() {
        let value = if arg == "-F" || arg == "--body-file" {
            args.next()?
        } else if let Some(glued) = arg.strip_prefix("--body-file=") {
            glued
        } else if supplies_a_body(arg) {
            return None;
        } else {
            continue;
        };
        if value == "-"
            || file
                .replace(value)
                .is_some()
        {
            return None;
        }
    }
    file
}

fn supplies_a_body(arg: &str) -> bool {
    let long = BODY_FLAGS
        .iter()
        .any(|flag| {
            arg.strip_prefix(flag)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('='))
        });
    let short = arg.starts_with('-')
        && !arg.starts_with("--")
        && arg
            .chars()
            .any(|c| BODY_SHORTS.contains(c));
    long || short
}

/// A stage that runs `gh` or `glab` itself with a verb that publishes.
fn forge_write(stage: &str) -> Option<Write<'_>> {
    let program = forge(shell::program(stage)?)?;
    let args = shell::program_args(stage)?;
    let (group, verb) = verb_pair(program, &args)?;
    Some(Write {
        program,
        group,
        verb,
    })
}

fn forge(word: &str) -> Option<&'static str> {
    match word
        .rsplit('/')
        .next()?
    {
        "gh" => Some("gh"),
        "glab" => Some("glab"),
        _ => None,
    }
}

/// The publishing group and verb among a program's arguments. Read two ways, since
/// a miss here is a write nobody was asked about: the subcommand words with flags
/// dropped, then any group word directly followed by one of its verbs.
fn verb_pair<'a>(program: &str, args: &[&'a str]) -> Option<(&'a str, &'a str)> {
    let publishes = |group: &str, verb: &str| match program {
        "gh" => {
            matches!(group, "pr" | "issue") && matches!(verb, "create" | "new" | "edit" | "comment")
                || (group, verb) == ("pr", "review")
        }
        _ => {
            matches!(group, "mr" | "issue") && matches!(verb, "create" | "new" | "update" | "note")
        }
    };
    let words = shell::verb_words(args);
    let leading = match words[..] {
        [group, verb, ..] if publishes(group, verb) => Some((group, verb)),
        _ => None,
    };
    leading.or_else(|| {
        args.windows(2)
            .map(|pair| (pair[0], pair[1]))
            .find(|(group, verb)| publishes(group, verb))
    })
}

#[cfg(test)]
mod tests {
    use super::{body_file, forge_write};

    fn publishes(stage: &str) -> bool {
        forge_write(stage).is_some()
    }

    #[test]
    fn writes_under_the_users_name_are_recognized() {
        for stage in [
            "gh pr create --title x --body-file scratch/pr-body-x.md",
            "gh -R o/r issue comment 3 -b hi",
            "gh pr review 3 --approve",
            "gh pr edit 3 --title y",
            "/usr/bin/gh issue new -t x",
            "glab mr create -t x -d y",
            "glab mr note 3 -m hi",
            "glab issue update 3 -l bug",
            "glab --verbose mr create -t x",
        ] {
            assert!(publishes(stage), "{stage}");
        }
    }

    #[test]
    fn reads_and_bot_wrappers_are_not() {
        for stage in [
            "gh pr view 3",
            "gh pr list --search create",
            "gh issue list",
            "glab mr list",
            "glab-cauca mr create -t x -d y",
            "glab-ng911 mr note 3 -m hi",
            "git commit -m 'gh pr create'",
        ] {
            assert!(!publishes(stage), "{stage}");
        }
    }

    #[test]
    fn a_create_names_exactly_one_body_file() {
        let args = |line: &'static str| -> Vec<&'static str> {
            line.split_whitespace()
                .collect()
        };
        assert_eq!(
            body_file(&args("pr create -t Q --body-file scratch/pr-body-x.md")),
            Some("scratch/pr-body-x.md")
        );
        assert_eq!(
            body_file(&args("pr create -F pr-body.md -d -l bug")),
            Some("pr-body.md")
        );
        for refused in [
            "pr create -t Q",
            "pr create -t Q --body Q",
            "pr create -t Q -b Q",
            "pr create --fill",
            "pr create -df",
            "pr create -F -",
            "pr create -F a.md --body-file=b.md",
            "pr create -F pr-body.md --template t.md",
            "pr create -F pr-body.md --web",
        ] {
            assert_eq!(body_file(&args(refused)), None, "{refused}");
        }
    }
}
