//! The rules a program can decide. Anything countable belongs here rather than in
//! the judge's prompt: the model reads prose well and counts badly.

use crate::output::HookOutput;

/// Past this a section is padding rather than a decision. Blank lines are not
/// counted, so the bound tracks what was written and not how it was spaced.
const MAX_SECTION_LINES: usize = 20;

const WHY_HEADING: &str = "A heading names the topic, not the question: `## Typed queries over \
raw document construction`, never `## Why …`. Rewrite the heading as the decision it records.";

const TOO_LONG: &str = "Section past the length bound — that is fluff, not thoroughness. Cut to \
the decision and the failure mode that drove it; a few sentences is the target.";

const CLAUDE_MD: &str = "design-rationale.md never references CLAUDE.md. State the constraint \
itself, so the section stands on its own for a reader holding only this repo.";

pub fn check(added: &str) -> Option<HookOutput> {
    if let Some(heading) = why_heading(added) {
        return Some(deny(WHY_HEADING, &heading));
    }
    if let Some((heading, lines)) = overlong_section(added) {
        return Some(deny(TOO_LONG, &format!("{heading} — {lines} lines")));
    }
    if added.contains("CLAUDE.md") {
        return Some(deny(CLAUDE_MD, "CLAUDE.md"));
    }
    None
}

fn deny(rule: &str, offending: &str) -> HookOutput {
    HookOutput::deny("PreToolUse", &format!("{rule}\n\nHere: {offending}"))
}

fn why_heading(added: &str) -> Option<String> {
    headings(added)
        .find(|heading| {
            let title = heading
                .trim_start_matches('#')
                .trim()
                .to_ascii_lowercase();
            title == "why" || title.starts_with("why ")
        })
        .map(str::to_string)
}

/// The first section over the bound, with the count. Text before any heading is a
/// section too — a body appended under a heading that already exists arrives with
/// no heading of its own and must still be measured.
fn overlong_section(added: &str) -> Option<(String, usize)> {
    let mut heading = String::from("(the text added, which names no heading)");
    let mut lines = 0;
    for line in added.lines() {
        if is_heading(line) {
            if lines > MAX_SECTION_LINES {
                return Some((heading, lines));
            }
            heading = line.to_string();
            lines = 0;
            continue;
        }
        if !line
            .trim()
            .is_empty()
        {
            lines += 1;
        }
    }
    (lines > MAX_SECTION_LINES).then_some((heading, lines))
}

fn headings(added: &str) -> impl Iterator<Item = &str> {
    added
        .lines()
        .filter(|line| is_heading(line))
}

/// A section heading, not a deeper one and not a `#` inside a fenced block — the
/// file's sections are all one level, so anything else is body text.
fn is_heading(line: &str) -> bool {
    line.starts_with("## ") && !line.starts_with("### ")
}
