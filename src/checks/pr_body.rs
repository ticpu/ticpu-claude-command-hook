//! The file a public PR or issue body is written to, checked as it is written so
//! its author corrects it before the user reads it.
//!
//! Only what a program can count is checked: a generated-with line, an emoji, a
//! reference-style link (an unmatched label degrades silently to brackets, and a
//! body posts once), prose hard-wrapped at a column (the forge reflows it, and
//! fixed wrapping breaks badly on a phone), and for a PR a heading saying how the
//! change was tested. The file is judged as the call would leave it, every fault
//! in one refusal, since a fragment cannot show a wrapped paragraph or a missing
//! section.
//!
//! `forge_write` reads the same faults when the create command runs, which is
//! what covers a file written through the shell or edited by hand.

use std::sync::LazyLock;

use regex::Regex;

use crate::checks::attribution;
use crate::checks::edited;
use crate::input::HookInput;
use crate::output::HookOutput;

static EMOJI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\p{Emoji_Presentation}|\x{FE0F}").expect("literal"));
static LINK_DEFINITION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^ {0,3}\[[^\]^][^\]]*\]:\s*\S").expect("literal"));
static LIST_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*([-*+]|\d+[.)])\s").expect("literal"));

/// Below this a line followed by another is a stacked pair, not a wrapped one.
const WRAPPED_FROM: usize = 40;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Pr,
    Issue,
}

/// `pr-body*.md` or `issue-body*.md`, wherever it sits.
pub fn kind(path: &str) -> Option<Kind> {
    let name = path
        .rsplit('/')
        .next()?
        .strip_suffix(".md")?;
    if name.starts_with("pr-body") {
        Some(Kind::Pr)
    } else if name.starts_with("issue-body") {
        Some(Kind::Issue)
    } else {
        None
    }
}

pub fn pre_tool_use(input: &HookInput) -> Option<HookOutput> {
    let kind = kind(input.file_path())?;
    let text = edited::after(input, &edited::before(input));
    let faults = faults(&text, kind);
    (!faults.is_empty()).then(|| HookOutput::deny("PreToolUse", &refusal(&faults)))
}

pub fn refusal(faults: &[String]) -> String {
    format!(
        "This body is read by the user and then posted as written. Fix before it lands:\n- {}",
        faults.join("\n- ")
    )
}

pub fn faults(text: &str, kind: Kind) -> Vec<String> {
    let mut faults = Vec::new();
    let mut fenced = false;
    let mut tested = false;
    let mut wrapped_here = false;
    let lines: Vec<&str> = text
        .lines()
        .collect();
    for (i, line) in lines
        .iter()
        .enumerate()
    {
        let n = i + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            wrapped_here = false;
            continue;
        }
        if EMOJI.is_match(line) {
            faults.push(format!("line {n}: emoji"));
        }
        if fenced {
            continue;
        }
        if attribution::carries_attribution(line) {
            faults.push(format!("line {n}: generated-with line"));
        }
        if LINK_DEFINITION.is_match(line) {
            faults.push(format!(
                "line {n}: reference-style link — write it inline as [text](url)"
            ));
        }
        tested |= trimmed.starts_with('#')
            && trimmed
                .to_lowercase()
                .contains("test");
        if trimmed.is_empty() {
            wrapped_here = false;
            continue;
        }
        let continues = lines
            .get(i + 1)
            .is_some_and(|next| continues_a_paragraph(next));
        if !wrapped_here && continues && is_wrapped_prose(line) {
            wrapped_here = true;
            faults.push(format!(
                "line {n}: paragraph hard-wrapped — one paragraph, one line"
            ));
        }
    }
    if kind == Kind::Pr && !tested {
        faults.push(
            "no testing section — add a heading saying how the change was tested, or that it \
             was not"
                .to_string(),
        );
    }
    faults
}

/// A prose or list-item line long enough to have been broken at a column, and not
/// ended by a Markdown hard break.
fn is_wrapped_prose(line: &str) -> bool {
    let trimmed = line.trim_start();
    !is_structural(trimmed)
        && line.len() - trimmed.len() < 4
        && trimmed
            .chars()
            .count()
            >= WRAPPED_FROM
        && !line.ends_with("  ")
        && !line.ends_with('\\')
}

/// The next line belongs to the paragraph or item above it.
fn continues_a_paragraph(next: &str) -> bool {
    let trimmed = next.trim_start();
    !trimmed.is_empty()
        && !is_structural(trimmed)
        && !LIST_ITEM.is_match(next)
        && !trimmed.starts_with("```")
        && !trimmed.starts_with("~~~")
}

/// A heading, table row, quote or HTML line: never prose to reflow.
fn is_structural(trimmed: &str) -> bool {
    trimmed.starts_with(['#', '|', '>', '<'])
}

#[cfg(test)]
mod tests {
    use super::Kind::{Issue, Pr};
    use super::{faults, kind};

    #[test]
    fn body_files_are_recognized_by_name() {
        assert!(kind("scratch/pr-body-fold.md") == Some(Pr));
        assert!(kind("/x/pr-body.md") == Some(Pr));
        assert!(kind("scratch/issue-body-fold.md") == Some(Issue));
        assert!(kind("docs/body.md").is_none());
        assert!(kind("pr-body.txt").is_none());
    }

    #[test]
    fn a_clean_body_has_no_fault() {
        let text = "## What\n\nOne paragraph on one line, however long it runs past any column anyone would wrap at.\n\n- an item that is long enough to have been wrapped but was not, and stays on its line\n- another\n\n```\na code line\nanother code line that is long enough to look like wrapped prose to a careless check\n```\n\n## Testing\n\ncargo test.\n\nCloses #1\nRefs #2\n";
        assert_eq!(faults(text, Pr), Vec::<String>::new());
    }

    #[test]
    fn every_fault_is_reported_with_its_line() {
        let text = "## What\n\nA paragraph that was wrapped at a fixed column by\nwhoever wrote it.\n\nSee [the doc][doc] ✅\n\n[doc]: https://example.test/doc\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n";
        let found = faults(text, Pr);
        for expected in [
            "line 3: paragraph hard-wrapped",
            "line 6: emoji",
            "line 8: reference-style link",
            "line 10: generated-with line",
            "no testing section",
        ] {
            assert!(
                found
                    .iter()
                    .any(|fault| fault.starts_with(expected)),
                "{expected}: {found:?}"
            );
        }
    }

    #[test]
    fn an_issue_needs_no_testing_section() {
        assert_eq!(
            faults("## Problem\n\nIt breaks.\n", Issue),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_wrapped_paragraph_is_reported_once() {
        let line = "a line of prose long enough to count as wrapped text";
        let text = format!("{line}\n{line}\n{line}\n\n## Tests\n");
        assert_eq!(faults(&text, Pr).len(), 1);
    }
}
