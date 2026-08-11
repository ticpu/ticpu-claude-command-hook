use super::judge::{headline, parse_for_test};
use super::mechanical::check;
use super::{FLOOR, framing, introduced, is_rationale, new_text};

/// Every finding is checked against the text it quotes, so a probe reply needs one
/// the text really contains.
const JUDGED: &str = "The cost of a single parser is one evasion instead of five, and an \
earlier version keyed the map on the identifier alone. The rule 3 steps are enumerated.";

fn objection(reply: &str) -> String {
    parse_for_test(reply, JUDGED).expect("findings")
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
    for number in 1..=5 {
        let rule = headline(number).unwrap_or_else(|| panic!("rule {number} has no headline"));
        assert!(!rule.is_empty());
        assert!(!rule.contains('.'), "{rule}");
    }
    assert!(headline(0).is_none());
    assert!(headline(6).is_none());
}

/// The model writes the finding line, so the citation is read tolerantly — but
/// only where it opens the line, or the word inside a quoted passage would name a
/// rule the passage has nothing to do with.
#[test]
fn a_cited_rule_is_named_beside_the_line_that_cited_it() {
    let reason = objection("REVISE\nRule 5: \"the cost of a single parser is\"");
    assert!(
        reason.contains("Rule 5: \"the cost of a single parser is\""),
        "{reason}"
    );
    assert!(reason.contains("NO SPEC RESTATEMENT"), "{reason}");

    for line in [
        "- **Rule 4**: \"one evasion instead of five\"",
        "rule #4 — \"one evasion\"",
    ] {
        let reason = objection(&format!("REVISE\n{line}"));
        assert!(reason.contains("NO ENUMERATED VALUES"), "{line}: {reason}");
    }

    // The number belongs to the quoted text, so the line cites nothing and is not a
    // finding at all — never a finding annotated with a rule it has nothing to do with.
    assert_eq!(
        parse_for_test(
            "REVISE\nThis says the \"rule 3 steps\" are enumerated.",
            JUDGED
        ),
        None
    );
}

/// A model that reasons in the open answers with a verdict and then argues itself to
/// the other one, quoting the passage on every line. None of those lines cites a rule,
/// and serving them back as an objection denies on the model's own deliberation.
#[test]
fn deliberation_is_not_a_finding() {
    let added = "One unknown-key walk, one deprecated-key list and one permission check cover \
both files.";
    let deliberating = "REVISE\n\
The text says: \"one deprecated-key list\".\n\
Wait, I see a potential Rule 4 violation here.\n\
I will pass because I cannot find a clear violation of the rules provided.";
    assert_eq!(parse_for_test(deliberating, added), None);

    // A line that does cite one is still a finding.
    assert!(parse_for_test("REVISE\nRule 4: \"one deprecated-key list\"", added).is_some());
}

/// The judged rules are the model's call; these two are refusals of a finding the
/// quoted passage cannot support at all.
#[test]
fn a_finding_its_quote_cannot_carry_is_dropped() {
    let added = "The loop belongs to the binary because only there does a failed pass keep \
its error chain and still leave the next pass scheduled.";

    // Nothing in that sentence refers to a previous state, so rule 3 cannot be it —
    // this is the shape the model reaches for whenever a passage orders or contrasts.
    assert_eq!(
        parse_for_test(
            "REVISE\nRule 3: \"The loop belongs to the binary\" (compared to an alternative)",
            added
        ),
        None
    );
    // Another rule against the same passage is untouched.
    assert!(parse_for_test("REVISE\nRule 1: \"The loop belongs to the binary\"", added).is_some());
    // A quote the text does not contain leaves nothing to rewrite.
    assert_eq!(
        parse_for_test(
            "REVISE\nRule 1: \"a sentence from another document\"",
            added
        ),
        None
    );
    // A finding quoting nothing cannot be checked or acted on.
    assert_eq!(
        parse_for_test("REVISE\nRule 1: the passage is generic", added),
        None
    );
    // Every finding dropped is a pass, not an empty objection.
    assert_eq!(
        parse_for_test("REVISE\nRule 3: \"belongs to the binary\"", added),
        None
    );

    // The rule still fires where the passage really does narrate one.
    let narrated = "An earlier version keyed the map on the identifier alone.";
    assert!(parse_for_test(&format!("REVISE\nRule 3: \"{narrated}\""), narrated).is_some());
}

/// An edit is judged on what it introduces: a removal re-emits the text around what
/// it takes out, and that text is already in the file and already approved. Whole
/// lines strip away entirely; a line the edit rewrote is the most that survives, and
/// never the section it sits in.
#[test]
fn a_removal_introduces_at_most_the_line_it_rewrote() {
    let section = "## A decision\n\nA first sentence that stands.\nA second that goes away \
because it repeated the first at greater length.\nA third that stays.\n";
    let shortened = "## A decision\n\nA first sentence that stands.\nA third that stays.\n";
    assert_eq!(new_text(section, shortened), "");
    assert!(
        new_text(section, shortened)
            .trim()
            .len()
            < FLOOR
    );

    // Cutting inside a line leaves that line, and nothing around it.
    let trimmed = "## A decision\n\nA first sentence.\nA second that goes away \
because it repeated the first at greater length.\nA third that stays.\n";
    assert_eq!(new_text(section, trimmed), "A first sentence.\n");
}

/// An insert lands before an existing heading and re-emits it, so the two texts share
/// that heading's marker. Stripping inside the line takes the marker with it, and the
/// new section's own heading then reaches the judge as a flat claim about the world.
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

/// Re-wrapping a paragraph rewrites every line of it and says nothing new, so the
/// rules must not be applied afresh to prose that has already been through them.
#[test]
fn a_reflow_introduces_nothing() {
    let wrapped = "A section named as already owning the decision may not be one the edit is\n\
rewriting or deleting, for the same reason.";
    let reflowed = "A section named as already owning the decision may not be one\nthe edit is \
rewriting or deleting, for the same reason.";
    assert_eq!(introduced(wrapped, reflowed), "");

    // A word added while re-wrapping is still an edit, and still judged.
    let with_a_change = "A section named as already owning the decision may never be one\nthe \
edit is rewriting or deleting, for the same reason.";
    assert!(!introduced(wrapped, with_a_change).is_empty());
}

/// A first section is judged with no document behind it, so the rules asking what a
/// reader of the repo would already know have nothing to read. Narrowing the passage
/// is then the one repair that cannot work, and the objection has to say so.
#[test]
fn only_a_first_section_is_asked_for_its_frame() {
    assert!(framing("").contains("does not exist yet"));
    assert!(framing("\n\n").contains("does not exist yet"));
    assert!(
        framing("# Design rationale\n\n## A decision\n\nBody.\n").is_empty(),
        "a document that exists reads against itself"
    );
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
