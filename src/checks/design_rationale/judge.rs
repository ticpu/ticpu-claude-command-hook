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
    Ok(findings(
        &super::ollama::ask(&prompt(document, replaced, added))?,
        added,
    ))
}

/// A passage narrating a previous state has to refer to one. Qualifying the rule's
/// own text does not hold the model to it — pressed for an answer it reaches for
/// whichever rule is nearest, and this is the nearest one for any passage that
/// contrasts, orders or chooses — so the precondition is checked here instead.
const PAST_REFERENCE: &[&str] = &[
    "used to",
    "previously",
    "earlier",
    "formerly",
    "originally",
    "initially",
    "no longer",
    "in the past",
    "prior to",
    "legacy",
    "was ",
    "were ",
    "had ",
    "been ",
];

/// The rule whose findings carry that precondition.
const PREVIOUS_STATE: u32 = 3;

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
fn findings(reply: &str, added: &str) -> Option<String> {
    let body = reply.trim();
    let (first, rest) = body
        .split_once('\n')
        .unwrap_or((body, ""));
    if !first
        .trim()
        .trim_end_matches(['.', ':'])
        .eq_ignore_ascii_case("REVISE")
    {
        return None;
    }
    let kept: Vec<&str> = rest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| stands(line, added))
        .collect();
    (!kept.is_empty()).then(|| annotate(&kept.join("\n")))
}

/// Whether the quoted passage can carry the finding at all. The quote has to be in
/// the text being judged — one that is not leaves nothing to rewrite — and a finding
/// against the previous-state rule has to quote a passage that refers to the past.
/// Both are refusals of an impossible finding, never a second opinion on a possible
/// one: what survives is still the model's call.
fn stands(line: &str, added: &str) -> bool {
    let Some(quote) = quoted(line) else {
        return false;
    };
    // Case-insensitive: a passage quoted out of the middle of a sentence comes back
    // with its first letter however the model felt like spelling it.
    let quote = super::collapsed(quote).to_ascii_lowercase();
    if !super::collapsed(added)
        .to_ascii_lowercase()
        .contains(&quote)
    {
        return false;
    }
    cited_rule(line) != Some(PREVIOUS_STATE)
        || PAST_REFERENCE
            .iter()
            .any(|marker| quote.contains(marker))
}

/// The passage a finding quotes, between the outermost quotation marks on its line.
/// A finding that quotes nothing is one there is no way to check or to act on.
fn quoted(line: &str) -> Option<&str> {
    let open = line.find('"')? + 1;
    let close = line.rfind('"')?;
    (close > open).then(|| &line[open..close])
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
pub(super) fn parse_for_test(reply: &str, added: &str) -> Option<String> {
    findings(reply, added)
}
