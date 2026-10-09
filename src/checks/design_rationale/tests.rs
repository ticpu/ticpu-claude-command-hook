use serde_json::json;

use super::mechanical::check;
use super::{introduced, is_rationale, new_text, post_tool_use, pre_tool_use};
use crate::input::HookInput;

fn edit(file_path: &str) -> HookInput {
    HookInput {
        hook_event_name: "PostToolUse".to_string(),
        tool_name: "Edit".to_string(),
        tool_input: json!({ "file_path": file_path }),
        ..HookInput::default()
    }
}

fn written(file_path: &str, content: &str) -> HookInput {
    HookInput {
        hook_event_name: "PreToolUse".to_string(),
        tool_name: "Write".to_string(),
        tool_input: json!({ "file_path": file_path, "content": content }),
        ..HookInput::default()
    }
}

fn denied(added: &str) -> bool {
    check(added).is_some()
}

fn reason(added: &str) -> String {
    let output = check(added).expect("expected a deny");
    output
        .hook_specific_output
        .and_then(|h| h.permission_decision_reason)
        .expect("a deny carries a reason")
}

#[test]
fn fires_only_on_a_rationale_file_under_docs() {
    assert!(is_rationale("docs/design-rationale.md"));
    assert!(is_rationale("/abs/crate/docs/design-rationale.md"));
    assert!(!is_rationale(
        "/home/x/.claude/commands/design-rationale.md"
    ));
    assert!(!is_rationale("design-rationale.md"));
    assert!(!is_rationale("docs/not-design-rationale.md"));
    assert!(!is_rationale("src/main.rs"));
    assert!(!is_rationale(""));
}

#[test]
fn a_why_heading_is_denied_and_quoted() {
    assert!(denied("## Why not llm-kit-anthropic\n\nBody.\n"));
    assert!(denied("## why we split the parser\n\nBody.\n"));
    let reason = reason("## Why investigation, not pattern matching\n\nBody.\n");
    assert!(
        reason.contains("## Why investigation, not pattern matching"),
        "{reason}"
    );
}

#[test]
fn a_heading_naming_its_topic_passes() {
    for added in [
        "## Typed queries over raw document construction\n\nBody.\n",
        // Only the question form is refused; the word itself is not.
        "## Whyless naming\n\nBody.\n",
        "## Addresses stay addresses, not tokens\n\nBody.\n",
    ] {
        assert!(!denied(added), "{added}");
    }
}

#[test]
fn a_claude_md_reference_is_denied() {
    assert!(denied(
        "## A rule worth stating\n\nCLAUDE.md already says so.\n"
    ));
    assert!(!denied("## A rule worth stating\n\nIt stands alone.\n"));
}

/// Blank lines are spacing, not content, so they must not push a short section
/// over the bound.
#[test]
fn length_counts_what_was_written_not_how_it_was_spaced() {
    let padded = format!("## A short section\n{}", "\n".repeat(60));
    assert!(!denied(&padded), "blank lines are not content");

    let long = format!(
        "## A long section\n\n{}",
        "A sentence of prose. ".repeat(90)
    );
    assert!(denied(&long));
    assert!(reason(&long).contains("## A long section"));
}

/// The wrap width is the author's: a file reflowed narrower says the same thing,
/// and a bound that moved with it would deny sections it had passed.
#[test]
fn a_reflow_does_not_change_the_verdict() {
    for width in [40, 72, 100, 400] {
        let wrapped = |text: &str| {
            text.split_whitespace()
                .fold(String::new(), |mut out, word| {
                    let last = out
                        .rsplit('\n')
                        .next()
                        .unwrap_or("")
                        .len();
                    if last + word.len() > width {
                        out.push('\n');
                    } else if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(word);
                    out
                })
        };

        let short = format!("## A section\n\n{}", wrapped(&"Short prose. ".repeat(20)));
        assert!(!denied(&short), "width {width} denied a short section");

        let long = format!(
            "## A section\n\n{}",
            wrapped(&"A sentence of prose. ".repeat(90))
        );
        assert!(denied(&long), "width {width} passed a long section");
    }
}

/// A body appended under a heading that already exists arrives with no heading of
/// its own, and still has to be measured.
#[test]
fn a_headless_body_is_measured_too() {
    let long = "A sentence of prose.\n".repeat(90);
    assert!(denied(&long));

    let short = "A sentence of prose.\n".repeat(3);
    assert!(!denied(&short));
}

/// The prompt names the headings an edit adds, so the text it copies back out of the
/// document to place itself must not be counted as added.
#[test]
fn a_removal_introduces_at_most_the_line_it_rewrote() {
    let section = "## A decision\n\nA first sentence that stands.\nA second that goes away \
because it repeated the first at greater length.\nA third that stays.\n";
    let shortened = "## A decision\n\nA first sentence that stands.\nA third that stays.\n";
    assert_eq!(new_text(section, shortened), "");

    // Cutting inside a line leaves that line, and nothing around it.
    let trimmed = "## A decision\n\nA first sentence.\nA second that goes away \
because it repeated the first at greater length.\nA third that stays.\n";
    assert_eq!(new_text(section, trimmed), "A first sentence.\n");
}

/// An insert lands before an existing heading and re-emits it, so the two texts share
/// that heading's marker. Stripping inside the line takes the marker with it.
#[test]
fn an_insert_before_a_heading_keeps_its_own_marker() {
    let replaced = "## A link reason is evidence\n";
    let added = "## The index build is a loop, not a timer\n\nBody of it.\n\n\
## A link reason is evidence\n";
    let introduced = new_text(replaced, added);
    assert!(
        introduced.starts_with("## The index build"),
        "heading lost its marker: {introduced:?}"
    );
    assert!(!introduced.contains("## A link reason"), "{introduced:?}");
}

/// Whole lines, so a slice never lands inside a multi-byte character.
#[test]
fn a_shared_line_is_stripped_only_as_a_whole() {
    assert_eq!(new_text("a — b\n", "a — b\nc — d\n"), "c — d\n");
    // Sharing only part of a line strips nothing: the line differs.
    assert_eq!(
        new_text("head tail", "head MIDDLE tail"),
        "head MIDDLE tail"
    );
    assert_eq!(new_text("", "all of it"), "all of it");
    assert_eq!(new_text("same", "same"), "");
}

#[test]
fn a_reflow_introduces_nothing() {
    let wrapped = "A section named as already owning the decision may not be one the edit is\n\
rewriting or deleting, for the same reason.";
    let reflowed = "A section named as already owning the decision may not be one\nthe edit is \
rewriting or deleting, for the same reason.";
    assert_eq!(introduced(wrapped, reflowed), "");

    let with_a_change = "A section named as already owning the decision may never be one\nthe \
edit is rewriting or deleting, for the same reason.";
    assert!(!introduced(wrapped, with_a_change).is_empty());
}

/// The gate reviewed it, and the writer has to be told so or it stops and asks for a
/// second review. It travels after the write: a permission prompt's reason is written
/// for whoever answers the prompt, which the writer only reads when it refused them.
#[test]
fn the_write_says_it_was_already_reviewed() {
    let advice = post_tool_use(&edit("docs/design-rationale.md")).expect("advice");
    let context = advice
        .hook_specific_output
        .and_then(|h| h.additional_context)
        .expect("carried as context, never as a decision");
    assert!(context.contains("was the review"), "{context}");

    // Every other Edit and Write goes through this same entry point.
    assert!(post_tool_use(&edit("src/main.rs")).is_none());
}

fn decision(output: crate::output::HookOutput) -> (String, String) {
    let specific = output
        .hook_specific_output
        .expect("a decision");
    (
        specific
            .permission_decision
            .expect("a decision"),
        specific
            .permission_decision_reason
            .expect("a reason"),
    )
}

/// A rationale that does not exist yet is the reader's alone: the whole file is in
/// front of them at the prompt, and no gate here has anything to read it against.
#[test]
fn a_document_that_does_not_exist_reaches_no_gate() {
    let added = "## Why we split the parser\n\nA body.\n";
    assert!(pre_tool_use(&written("/x/docs/design-rationale.md", added)).is_none());
    // The same text against a file that exists is the countable rules' business.
    let here = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/design-rationale.md");
    let (verdict, _) = decision(pre_tool_use(&written(here, added)).expect("gated"));
    assert_eq!(verdict, "deny");
}

/// Past the countable rules every edit is the user's to read, however small, and
/// the prompt says where it lands.
#[test]
fn an_edit_is_prompted_with_its_placement() {
    let here = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/design-rationale.md");
    let added = "# Design rationale\n\n## A decision\n\nBody.\n";
    let (verdict, reason) = decision(pre_tool_use(&written(here, added)).expect("gated"));
    assert_eq!(verdict, "ask");
    assert!(reason.contains("Replaces the whole file"), "{reason}");
    assert!(reason.contains("## A decision"), "{reason}");
}
