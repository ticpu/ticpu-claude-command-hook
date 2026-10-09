//! The gate on `docs/design-rationale.md`: an edit is prompted, never judged.
//!
//! An edit waits on a whole read of the file since the session last compacted
//! (`read_whole`). A rule a program can count refuses outright, having nothing to
//! weigh (`mechanical`). Every other edit is forced to a prompt naming the section
//! it lands in and the headings it adds, which an Edit's diff never shows
//! (`placement`). Approving that prompt is the review, and the write says so
//! afterwards: a prompt's reason is addressed to whoever answers it, and a writer
//! not told stops to ask for a second review nobody owes.
//!
//! A file that does not exist yet reaches no gate and takes the normal prompt.
//! Only a file under a `docs/` directory is matched, so a command or skill file
//! of the same name is left alone.

pub mod disabled;
mod mechanical;
mod placement;
pub mod read_whole;
pub mod shell_write;
#[cfg(test)]
mod tests;

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::input::HookInput;
use crate::output::HookOutput;

const REVIEW: &str = "design-rationale.md — approving this is the review. Reject to say what \
should change.";

pub fn pre_tool_use(input: &HookInput) -> Option<HookOutput> {
    let path = input.file_path();
    if !is_rationale(path) {
        return None;
    }
    // Ahead of every gate below, including the countable rules: the switch is off or
    // it is not, and a rule still firing under it is one the user did not turn off.
    if disabled::active() {
        return Some(disabled::notice());
    }
    let document = read_document(path)?;
    if let Some(refused) = read_whole::gate(input) {
        return Some(refused);
    }
    // A `Write` replaces the whole file, so what it takes out is what is on disk.
    let (replaced, added) = match input
        .tool_name
        .as_str()
    {
        "Write" => (document.as_str(), input.content()),
        _ => (input.old_string(), input.new_string()),
    };
    // The countable rules measure the whole replacement: a section left over the
    // length bound is over it however much this edit trimmed.
    if let Some(refused) = mechanical::check(added) {
        return Some(refused);
    }
    Some(HookOutput::ask(
        "PreToolUse",
        &format!(
            "{REVIEW}\n\n{}",
            placement::describe(&document, replaced, introduced(replaced, added))
        ),
    ))
}

/// What the edit says that the document did not, which is nothing at all when it only
/// re-wrapped what was there.
fn introduced<'a>(replaced: &str, added: &'a str) -> &'a str {
    match collapsed(replaced) == collapsed(added) {
        true => "",
        false => new_text(replaced, added),
    }
}

/// What the edit introduces, with the whole lines shared at both ends stripped. An
/// edit appending a section carries an anchor copied out of the document, and one
/// editing a section in place carries whatever it leaves standing around the change.
///
/// Whole lines only: an edit inserting a section before an existing one shares that
/// heading's marker, and a strip running inside the line would take it.
fn new_text<'a>(replaced: &str, added: &'a str) -> &'a str {
    let (old, new) = (lines(replaced), lines(added));
    let head: usize = common(old.iter(), new.iter());
    // Never past what the head already claimed, or a line counts at both ends.
    let tail: usize = common(
        old[head..]
            .iter()
            .rev(),
        new[head..]
            .iter()
            .rev(),
    );
    let start = new[..head]
        .iter()
        .map(|line| line.len())
        .sum();
    let end = new[new.len() - tail..]
        .iter()
        .map(|line| line.len())
        .sum::<usize>();
    &added[start..added.len() - end]
}

/// Lines with their terminators kept, so the pieces re-assemble into the original.
fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n')
        .collect()
}

fn common<'a>(a: impl Iterator<Item = &'a &'a str>, b: impl Iterator<Item = &'a &'a str>) -> usize {
    a.zip(b)
        .take_while(|(x, y)| x == y)
        .count()
}

/// The document is hard-wrapped, so a paragraph reflowed is the same paragraph.
fn collapsed(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

const REVIEWED: &str = "The design-rationale edit you just made was reviewed at the gate — the \
prompt you were shown was the review, and approving it was the verdict. Do not present the diff \
and ask for another review of it. Commit it and carry on.";

/// The write happened; the only thing left to say is who read it — which under the
/// standing switch is nobody.
pub fn post_tool_use(input: &HookInput) -> Option<HookOutput> {
    is_rationale(input.file_path()).then(|| {
        HookOutput::advise(
            "PostToolUse",
            match disabled::active() {
                true => disabled::UNREVIEWED,
                false => REVIEWED,
            },
        )
    })
}

fn is_rationale(file_path: &str) -> bool {
    let path = Path::new(file_path);
    path.file_name()
        .is_some_and(|name| name == "design-rationale.md")
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|dir| dir == "docs")
}

/// The file the edit lands in, or `None` when it does not exist yet. Any other read
/// failure is reported and answered with an empty document rather than the skip an
/// absent file gets.
fn read_document(path: &str) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("design_rationale: read {path} failed: {e}");
            Some(String::new())
        }
    }
}
