//! Whether the document already has a section owning this decision. Asked on its
//! own because the answer is a relation to the rest of the file rather than a fault
//! inside the new text: bundled with the rules, it is never the thing a model
//! reaches for, since every other rule is satisfied by quoting a bad passage.

use anyhow::Result;

const PREAMBLE: &str = "\
You are given a design-rationale document and a passage being added to it.

Find the section that ALREADY records the same decision as the new passage — the section it
should be folded into instead of standing beside. The same decision means the same constraint,
restated in other words. A section on a related subject, using similar words, or about the same
component for a different reason, is NOT the same decision.

Almost always there is none, and NONE is the answer. Only say otherwise if you can copy out an
existing sentence that already states what the new passage states. If you cannot find that
sentence, the answer is NONE. If you are weighing it, the answer is NONE.

Answer NONE on a line by itself, or exactly two lines: the heading copied verbatim starting
with ##, then the existing sentence copied verbatim. Nothing else.";

/// Short enough to appear anywhere is no evidence of anything.
const MIN_EVIDENCE: usize = 40;

pub(super) fn review(document: &str, replaced: &str, added: &str) -> Result<Option<String>> {
    let new = new_text(replaced, added);
    if new
        .trim()
        .len()
        < super::FLOOR
    {
        return Ok(None);
    }
    let reply = super::ollama::ask(&prompt(document, new))?;
    Ok(owner(document, &reply)
        .filter(|(heading, _)| Some(*heading) != landing_section(document, replaced, added))
        .map(|(heading, evidence)| {
            format!(
                "Already recorded under \"{heading}\" — fold it in there rather than adding a \
                 section:\n    \"{evidence}\""
            )
        }))
}

/// The named section has to contain a sentence saying what the new passage says,
/// and that sentence has to be in it. A model asked only to name a section names
/// one whenever the words overlap; asked for the sentence, it has to find one, and
/// a reader can weigh the objection instead of taking it.
fn owner<'a>(document: &'a str, reply: &str) -> Option<(&'a str, String)> {
    let mut lines = reply
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let named = lines.next()?;
    if !named.starts_with("##") {
        return None;
    }
    let heading = document
        .lines()
        .map(str::trim)
        .find(|line| *line == named)?;
    let evidence = lines
        .next()?
        .trim_matches(['"', '\''])
        .to_string();
    (evidence.len() >= MIN_EVIDENCE
        && collapsed(section(document, heading)).contains(&collapsed(&evidence)))
    .then_some((heading, evidence))
}

/// The body under a heading, up to the next one.
fn section<'a>(document: &'a str, heading: &str) -> &'a str {
    let Some(at) = document.find(heading) else {
        return "";
    };
    let body = &document[at + heading.len()..];
    match body.find("\n## ") {
        Some(end) => &body[..end],
        None => body,
    }
}

/// The document is hard-wrapped, so a sentence quoted back as one line is the same
/// sentence as one broken across two.
fn collapsed(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn prompt(document: &str, added: &str) -> String {
    format!("{PREAMBLE}\n\n=== DOCUMENT ===\n{document}\n\n=== NEW PASSAGE ===\n{added}\n")
}

/// What the edit actually introduces. An edit appending a section carries an anchor
/// copied out of the document, and that anchor is a perfect duplicate of the section
/// it came from — left in, it is the only thing the reviewer would ever report.
fn new_text<'a>(replaced: &str, added: &'a str) -> &'a str {
    let head = common_len(replaced.chars(), added.chars());
    let rest = &added[head..];
    let tail = common_len(
        replaced[head..]
            .chars()
            .rev(),
        rest.chars()
            .rev(),
    );
    &rest[..rest.len() - tail]
}

fn common_len(a: impl Iterator<Item = char>, b: impl Iterator<Item = char>) -> usize {
    a.zip(b)
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

/// The section the edit lands in, which can never be the section it duplicates —
/// it is either being rewritten, or added to, and "fold it in there" names where
/// the text is already going. Only text arriving under a heading of its own stands
/// as a separate section, and then the one holding the anchor is fair game.
fn landing_section<'a>(document: &'a str, replaced: &str, added: &str) -> Option<&'a str> {
    if replaced
        .trim()
        .is_empty()
        || (added.contains(replaced) && new_text(replaced, added).contains("## "))
    {
        return None;
    }
    document[..document.find(replaced)?]
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("## "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "# Design rationale\n\n## A first decision\n\nBody of the first.\n\n\
## A second decision\n\nBody of the second. The second is decided where the first\ncannot \
reach it.\n";

    #[test]
    fn an_appended_section_is_judged_without_its_anchor() {
        let replaced = "Body of the second.";
        let added = "Body of the second.\n\n## A third\n\nBody of the third.";
        assert_eq!(
            new_text(replaced, added),
            "\n\n## A third\n\nBody of the third."
        );
    }

    /// Common text at both ends is anchor, not content.
    #[test]
    fn text_shared_at_both_ends_is_stripped() {
        assert_eq!(new_text("head tail", "head MIDDLE tail"), "MIDDLE ");
        assert_eq!(new_text("", "all of it"), "all of it");
        assert_eq!(new_text("same", "same"), "");
    }

    /// An em dash must not be split; the document is full of them.
    #[test]
    fn multibyte_text_is_split_on_a_character_boundary() {
        assert_eq!(new_text("a — b", "a — c"), "c");
    }

    /// Both halves have to check out: a heading the document has, and a sentence
    /// that section really contains. Either one alone is a claim, not evidence.
    #[test]
    fn an_objection_needs_a_heading_and_a_sentence_that_is_really_there() {
        let quoted = "The second is decided where the first cannot reach it";
        assert_eq!(
            owner(DOC, &format!("## A second decision\n{quoted}")),
            Some(("## A second decision", quoted.to_string()))
        );
        // Quoted back as one line, though the document wraps it across two.
        assert!(
            owner(
                DOC,
                "## A second decision\n\"second is decided where the first cannot reach it\""
            )
            .is_some()
        );

        assert_eq!(owner(DOC, "NONE"), None);
        assert_eq!(owner(DOC, ""), None);
        // A verdict wrapped in prose is not one.
        assert_eq!(owner(DOC, "It belongs under ## A second decision"), None);
        // A heading with no sentence behind it.
        assert_eq!(owner(DOC, "## A second decision"), None);
        assert_eq!(
            owner(DOC, "## A section it invented\nBody of the first."),
            None
        );
        // The sentence exists, but not in the section named.
        assert_eq!(owner(DOC, &format!("## A first decision\n{quoted}")), None);
        // Too short to be evidence of anything.
        assert_eq!(owner(DOC, "## A second decision\nThe second is"), None);
    }

    #[test]
    fn a_section_being_written_into_is_not_a_section_being_duplicated() {
        let replaced = "Body of the second.";
        assert_eq!(
            landing_section(DOC, replaced, "Rewritten body."),
            Some("## A second decision")
        );
        // A heading of its own: the new text stands beside that section, not in it.
        assert_eq!(
            landing_section(DOC, replaced, "Body of the second.\n\n## A third\n\nMore."),
            None
        );
        assert_eq!(landing_section(DOC, "", "Anything."), None);
    }

    /// A sentence appended to a section is being folded into it already, so naming
    /// that section as the one to fold into is an objection with nothing to do.
    #[test]
    fn a_sentence_appended_to_a_section_lands_in_it() {
        let replaced = "Body of the second.";
        assert_eq!(
            landing_section(DOC, replaced, "Body of the second. And one sentence more."),
            Some("## A second decision")
        );
    }
}
