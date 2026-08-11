use super::judge::{headline, parse_for_test};
use super::mechanical::check;
use super::{FLOOR, is_rationale, post_tool_use};

fn objection(reply: &str) -> String {
    parse_for_test(reply).expect("findings")
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
fn fires_only_on_a_rationale_file() {
    assert!(is_rationale("docs/design-rationale.md"));
    assert!(is_rationale("/abs/crate/docs/design-rationale.md"));
    assert!(!is_rationale("src/main.rs"));
    assert!(!is_rationale("README.md"));
    assert!(!is_rationale(""));
    assert!(post_tool_use("docs/design-rationale.md").is_some());
    assert!(post_tool_use("README.md").is_none());
}

/// The review reminder must not ask for the diff back: the tool result already
/// rendered it, and repeating it is what this check exists to stop.
#[test]
fn the_reminder_does_not_ask_for_the_diff_again() {
    let output = post_tool_use("docs/design-rationale.md").expect("fires");
    let context = output
        .hook_specific_output
        .and_then(|h| h.additional_context)
        .expect("carries context");
    assert!(context.contains("Do not restate"), "{context}");
    assert!(!context.contains("present the diff"), "{context}");
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
        "A sentence of prose.\n".repeat(30)
    );
    assert!(denied(&long));
    assert!(reason(&long).contains("## A long section"));
}

/// A body appended under a heading that already exists arrives with no heading of
/// its own, and still has to be measured.
#[test]
fn a_headless_body_is_measured_too() {
    let long = "A sentence of prose.\n".repeat(30);
    assert!(denied(&long));

    let short = "A sentence of prose.\n".repeat(3);
    assert!(!denied(&short));
}

/// A number with no rule behind it must annotate nothing rather than guess, and
/// every number the list does carry has to resolve — the lookup reads `rules.md`,
/// so a reformat of that file would otherwise silently stop naming anything.
#[test]
fn every_listed_rule_resolves_and_nothing_else_does() {
    for number in 1..=6 {
        let rule = headline(number).unwrap_or_else(|| panic!("rule {number} has no headline"));
        assert!(!rule.is_empty());
        assert!(!rule.contains('.'), "{rule}");
    }
    assert!(headline(0).is_none());
    assert!(headline(7).is_none());
}

/// The model writes the finding line, so the citation is read tolerantly — but
/// only where it opens the line, or the word inside a quoted passage would name a
/// rule the passage has nothing to do with.
#[test]
fn a_cited_rule_is_named_beside_the_line_that_cited_it() {
    let reason = objection("REVISE\nRule 4: \"the cost of a single parser is\"");
    assert!(
        reason.contains("Rule 4: \"the cost of a single parser is\""),
        "{reason}"
    );
    assert!(reason.contains("NO DERIVABLE CONSEQUENCE"), "{reason}");

    for line in ["- **Rule 5**: \"x\"", "rule #5 — \"x\""] {
        let reason = objection(&format!("REVISE\n{line}"));
        assert!(reason.contains("NO ENUMERATED VALUES"), "{line}: {reason}");
    }

    // The number belongs to the quoted text, not to a citation.
    let reason = objection("REVISE\nThis says the rule 3 steps are enumerated.");
    assert!(!reason.contains("NO BEFORE"), "{reason}");
}

/// The floor exists to keep deletions and one-line fixes off the judge; it must
/// not be so high that a real section skips review.
#[test]
fn the_floor_sits_between_a_tweak_and_a_section() {
    let link_fix = "[EslEventType](event/event_type.rs)";
    assert!(link_fix.len() < FLOOR);

    let section = "## A link reason is evidence, recorded where it is observed\n\nA reason is \
recorded by whatever observed it, at the point it was observed, rather than reconstructed \
later from what happens to still be in scope.\n";
    assert!(section.len() >= FLOOR);
}
