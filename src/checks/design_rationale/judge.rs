//! The rules a reader has to decide, asked of a local model over ollama. Each one
//! is answerable by quoting a passage that is wrong on its own — a rule needing the
//! rest of the document to decide is asked separately, in `overlap`.

use anyhow::Result;

const RULES: &str = include_str!("rules.md");

const PREAMBLE: &str = "\
You are reviewing an edit to a file called design-rationale.md, against a short closed list of
rules. You are given the whole document as it stands, then the edit: the text being replaced,
and the text replacing it. Judge ONLY the replacement text.

MOST EDITS ARE FINE. The author knows these rules and follows them. Your job is not to find
something wrong — it is to catch the occasional edit that clearly breaks a listed rule. Answer
PASS unless you can point at a specific passage a careful editor would certainly cut. If you
are weighing whether something counts, it does not: answer PASS.

Your first line must be exactly PASS or exactly REVISE. Nothing else on that line.
If REVISE, follow it with ONE line per violation, at most two, each naming a rule NUMBER and
quoting the offending passage verbatim. Never quote the same passage twice, and never add a
second line to pad — one real violation beats one real and one invented.

Do not rewrite the text. Do not comment on style you merely dislike.";

/// `Ok(None)` is a pass. A reply that is not a verdict is a failed review, never a
/// deny: the model is not asked a question it could answer by blocking.
pub(super) fn review(document: &str, replaced: &str, added: &str) -> Result<Option<String>> {
    Ok(findings(&super::ollama::ask(&prompt(
        document, replaced, added,
    ))?))
}

fn prompt(document: &str, replaced: &str, added: &str) -> String {
    let replaced = if replaced
        .trim()
        .is_empty()
    {
        "(nothing — this edit only adds text)"
    } else {
        replaced
    };
    format!(
        "{PREAMBLE}\n\n=== RULES ===\n{RULES}\n\n=== DOCUMENT AS IT STANDS ===\n{document}\n\n\
         === TEXT BEING REPLACED ===\n{replaced}\n\n=== REPLACEMENT TEXT ===\n{added}\n"
    )
}

/// The first line decides. Anything else is a model that did not answer the
/// question asked, and a verdict with nothing to act on is worse than none: it
/// would cost a rewrite with no idea what to change.
fn findings(reply: &str) -> Option<String> {
    let body = reply.trim();
    let (first, rest) = body
        .split_once('\n')
        .unwrap_or((body, ""));
    match first
        .trim()
        .trim_end_matches(['.', ':'])
        .to_ascii_uppercase()
        .as_str()
    {
        "REVISE"
            if !rest
                .trim()
                .is_empty() =>
        {
            Some(annotate(rest.trim()))
        }
        _ => None,
    }
}

/// The rule as written, for a number the model cited. Asking the model to restate
/// the rule would be more generation under the padding pressure that invents
/// findings, so it is only ever asked for the number.
pub(super) fn headline(number: u32) -> Option<&'static str> {
    let body = RULES
        .lines()
        .find_map(|line| {
            line.trim_start()
                .strip_prefix(&format!("{number}. "))
        })?;
    let end = body.find('.')?;
    Some(&body[..end])
}

/// Tolerant of how the model spells a citation — bare, bulleted or bold — but only
/// where one opens the line, so the word inside a quoted passage is not read as one.
fn cited_rule(line: &str) -> Option<u32> {
    let at = line
        .to_ascii_lowercase()
        .find("rule")?;
    if line[..at]
        .chars()
        .any(char::is_alphanumeric)
    {
        return None;
    }
    line[at + 4..]
        .trim_start_matches([' ', ':', '#', '*'])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// The model's line is kept verbatim and the rule named beneath it: a number that
/// does not match the passage it quotes has to stay visible, not be dressed up.
fn annotate(findings: &str) -> String {
    findings
        .lines()
        .flat_map(|line| {
            let named = cited_rule(line)
                .and_then(headline)
                .map(|rule| format!("    ({rule})"));
            std::iter::once(line.to_string()).chain(named)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
pub(super) fn parse_for_test(reply: &str) -> Option<String> {
    findings(reply)
}
