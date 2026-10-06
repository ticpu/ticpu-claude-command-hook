//! Where an edit lands, said on the approval prompt: an Edit's prompt renders the diff
//! alone, and a subsection added under the section owning its decision reads there
//! like a section of its own.

pub(super) fn describe(document: &str, replaced: &str, introduced: &str) -> String {
    let under = match replaced == document {
        true => "Replaces the whole file.".to_string(),
        false => match document.find(replaced) {
            Some(at) if !replaced.is_empty() => match enclosing(&document[..at]) {
                Some(chain) => format!("Lands under {chain}."),
                None => "Lands before the first section.".to_string(),
            },
            _ => "Lands at a place the file does not contain.".to_string(),
        },
    };
    let added: Vec<&str> = headings(introduced).collect();
    match added.is_empty() {
        true => format!("{under} Adds no heading."),
        false => format!("{under} Adds {}.", added.join(", ")),
    }
}

/// The last `##` before the edit, and the `###` inside it the edit falls in, if any.
fn enclosing(before: &str) -> Option<String> {
    let mut section = None;
    let mut subsection = None;
    for line in headings(before) {
        match line.starts_with("### ") {
            true => subsection = Some(line),
            false => (section, subsection) = (Some(line), None),
        }
    }
    section.map(|section| match subsection {
        Some(subsection) => format!("{section} › {subsection}"),
        None => section.to_string(),
    })
}

/// `##` and `###` lines outside fenced blocks, where a `#` is a shell comment.
fn headings(text: &str) -> impl Iterator<Item = &str> {
    let mut fenced = false;
    text.lines()
        .filter(move |line| {
            if line.starts_with("```") {
                fenced = !fenced;
            }
            !fenced && (line.starts_with("## ") || line.starts_with("### "))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "# Design rationale\n\n## Storage\n\nBody.\n\n### Ordering\n\n\
Ordered body.\n\n## Parsing\n\n```sh\n## not a heading\n```\n\nParsing body.\n";

    #[test]
    fn an_edit_names_the_section_and_subsection_it_lands_in() {
        assert_eq!(
            describe(DOCUMENT, "Ordered body.", "Ordered body, and more.\n"),
            "Lands under ## Storage › ### Ordering. Adds no heading."
        );
        assert_eq!(
            describe(
                DOCUMENT,
                "Parsing body.",
                "Parsing body.\n\n### Keys\n\nText.\n"
            ),
            "Lands under ## Parsing. Adds ### Keys."
        );
    }

    /// An insertion anchored on a heading goes in front of it, so under the one before.
    #[test]
    fn an_edit_anchored_on_a_heading_lands_in_the_section_before_it() {
        assert_eq!(
            describe(DOCUMENT, "## Parsing", "## Lexing\n\nText.\n\n"),
            "Lands under ## Storage › ### Ordering. Adds ## Lexing."
        );
    }

    #[test]
    fn a_write_and_a_preamble_edit_say_so() {
        assert_eq!(
            describe(DOCUMENT, DOCUMENT, "## Storage\n"),
            "Replaces the whole file. Adds ## Storage."
        );
        assert_eq!(
            describe(DOCUMENT, "# Design rationale", "# Design rationale, again"),
            "Lands before the first section. Adds no heading."
        );
    }
}
