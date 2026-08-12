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
PASS unless you can point at a specific passage a careful editor would certainly cut.

Your first line must be exactly PASS, exactly REVISE, or exactly CONTEXT. Nothing else on that
line.

REVISE: a rule is broken and you can quote the passage that breaks it. Follow with ONE line per
violation, at most two, each naming a rule NUMBER and quoting the offending passage verbatim.
Never quote the same passage twice, and never add a second line to pad — one real violation
beats one real and one invented.

CONTEXT: a rule turns on something you were not told and cannot read off the text — whether a
component is this project's or a dependency's, whether a name is a config key here or a field
of some other program, what a reader holding this repository would already have, or whether the
passage would still inform a change someone could make later. Follow with ONE line, at most
two, each naming exactly what you would need to know. Ask for the missing fact, never for the
passage to be longer, and never as a way of hedging a violation you could quote — that is
REVISE.

PASS: it respects the rules, or you are weighing whether something counts.

Do not rewrite the text. Do not comment on style you merely dislike.";

/// What the review came back with. `Questions` is the verdict a small model can give
/// honestly where it would otherwise have to guess: the rules it holds are the ones
/// answerable by quoting the passage, and the ones that decide hardest — is this
/// component ours, would anyone face this decision again — turn on what the model was
/// never told. Both non-passing verdicts stop the edit, since both have something the
/// writer must act on and only a deny reaches them.
pub(super) enum Verdict {
    Pass,
    Findings(String),
    Questions(String),
}

/// A reply that is not a verdict is a failed review, never a deny: the model is not
/// asked a question it could answer by blocking.
pub(super) fn review(
    document: &str,
    replaced: &str,
    added: &str,
    context: &str,
) -> Result<Verdict> {
    Ok(read(
        &super::ollama::ask(&prompt(document, replaced, added, context))?,
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

/// A passage enumerating values has to name one. What the rule is for is a member
/// that goes stale when it is renamed or another is added, so a passage naming only
/// categories satisfies it — but it shares the rule's own vocabulary ("key", "list",
/// "check"), and a lexical match on that is what the finding usually is.
///
/// Read off the quote, which the model returns verbatim; a value it paraphrased into
/// prose is one the finding could not point at anyway. Underscores count beside code
/// spans, since a field named in running text loses its backticks before it loses
/// its own spelling.
fn names_a_value(quote: &str) -> bool {
    quote.contains('`')
        || quote.contains('_')
        || quote
            .chars()
            .any(|c| c.is_ascii_digit())
}

/// The rules whose findings carry a precondition.
const PREVIOUS_STATE: u32 = 3;
const ENUMERATED_VALUES: u32 = 4;

fn prompt(document: &str, replaced: &str, added: &str, context: &str) -> String {
    let replaced = if replaced
        .trim()
        .is_empty()
    {
        "(nothing — this edit only adds text)"
    } else {
        replaced
    };
    // Answers to a previous round's questions, from the author. Last, so the model
    // reads them holding the passage they are about.
    let context = match context.trim() {
        "" => String::new(),
        answers => format!("\n=== WHAT THE AUTHOR ANSWERED WHEN ASKED ===\n{answers}\n"),
    };
    format!(
        "{PREAMBLE}\n\n=== RULES ===\n{RULES}\n\n=== DOCUMENT AS IT STANDS ===\n{document}\n\n\
         === TEXT BEING REPLACED ===\n{replaced}\n\n=== REPLACEMENT TEXT ===\n{added}\n{context}"
    )
}

/// The first line decides. Anything else is a model that did not answer the
/// question asked, and a verdict with nothing to act on is worse than none: it
/// would cost a rewrite with no idea what to change.
///
/// The word opens the line but need not be all of it: told to answer on one line
/// and then to say what it needs, a model puts both there, and reading that as no
/// verdict at all passes exactly the edits it meant to stop.
fn read(reply: &str, added: &str) -> Verdict {
    let body = reply.trim();
    let (first, rest) = body
        .split_once('\n')
        .unwrap_or((body, ""));
    let (verdict, trailing) = split_verdict(first);
    if verdict.eq_ignore_ascii_case("CONTEXT") {
        return questions(&[trailing, rest].join("\n"));
    }
    if !verdict.eq_ignore_ascii_case("REVISE") {
        return Verdict::Pass;
    }
    let rest = &[trailing, rest].join("\n");
    let kept: Vec<&str> = rest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| stands(line, added))
        .collect();
    match kept.is_empty() {
        true => Verdict::Pass,
        false => Verdict::Findings(annotate(&kept.join("\n"))),
    }
}

/// The verdict word and whatever the model glued to it, which is a finding or a
/// question often enough that dropping it loses the answer.
fn split_verdict(line: &str) -> (&str, &str) {
    let line = line.trim();
    let end = line
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(line.len());
    let (word, rest) = line.split_at(end);
    (
        word,
        rest.trim_start_matches([':', '.', '-', '—', ' '])
            .trim(),
    )
}

/// The questions to put to the writer, at most the two the reply was allowed. A
/// verdict of CONTEXT naming nothing asks for nothing, and passes: what makes this
/// level worth having is the question, and a stop with no question attached is the
/// bare refusal it exists to replace.
fn questions(rest: &str) -> Verdict {
    let asked: Vec<&str> = rest
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(2)
        .collect();
    match asked.is_empty() {
        true => Verdict::Pass,
        false => Verdict::Questions(asked.join("\n")),
    }
}

/// Whether the quoted passage can carry the finding at all. A finding names a rule and
/// quotes the text being judged — a line doing neither is the model reasoning in the
/// open, which it does by answering with a verdict and then arguing itself to the
/// other one, quoting the passage on every line as it goes. A finding against the
/// previous-state rule has to quote a passage that refers to the past. All three are
/// refusals of an impossible finding, never a second opinion on a possible one: what
/// survives is still the model's call.
fn stands(line: &str, added: &str) -> bool {
    let Some(rule) = cited_rule(line) else {
        return false;
    };
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
    match rule {
        PREVIOUS_STATE => PAST_REFERENCE
            .iter()
            .any(|marker| quote.contains(marker)),
        ENUMERATED_VALUES => names_a_value(&quote),
        _ => true,
    }
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
    match read(reply, added) {
        Verdict::Findings(objection) => Some(objection),
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn questions_for_test(reply: &str, added: &str) -> Option<String> {
    match read(reply, added) {
        Verdict::Questions(asked) => Some(asked),
        _ => None,
    }
}
