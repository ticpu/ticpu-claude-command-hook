//! The rules only a reader can decide, asked of a local model over ollama.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::output::HookOutput;

const RULES: &str = include_str!("rules.md");

const MODEL_ENV: &str = "CLAUDE_HOOK_JUDGE_MODEL";
const URL_ENV: &str = "CLAUDE_HOOK_JUDGE_URL";
const DEFAULT_MODEL: &str = "gemma4:12b";
const DEFAULT_URL: &str = "http://localhost:11434/api/generate";

/// Stated on every request rather than taken from the server's configuration: a
/// short default truncates the document out of the prompt, and a truncated prompt
/// does not fail — the model answers from whatever survived.
const NUM_CTX: u32 = 32768;

/// Generous against a warm judge, and still inside the hook budget when the model
/// has to be loaded first.
const TIMEOUT: Duration = Duration::from_secs(45);

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

const DENY_HEAD: &str = "The design-rationale judge objects. Revise the section and re-issue \
the edit — or say why the objection is wrong and I will pass it on.";

/// Never a deny: an edit is not blocked because the reviewer could not be reached.
/// It is not a silent pass either — the model gets nothing, and the line below is
/// the user's only sign that nothing was reviewed.
fn unreviewed(why: &str) -> HookOutput {
    HookOutput::note(&format!("design-rationale judge did not run: {why}"))
}

pub fn check(document: &str, replaced: &str, added: &str) -> Option<HookOutput> {
    match ask(&prompt(document, replaced, added)) {
        Ok(reply) => verdict(&reply),
        Err(e) => Some(unreviewed(&format!("{e:#}"))),
    }
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

fn ask(prompt: &str) -> Result<String> {
    let model = var(MODEL_ENV, DEFAULT_MODEL);
    let url = var(URL_ENV, DEFAULT_URL);
    let body = json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "think": false,
        "options": { "temperature": 0, "num_ctx": NUM_CTX },
    });
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .post(&url)
        .send_json(&body)
        .with_context(|| format!("POST {url} ({model})"))?;
    let value: Value = response
        .body_mut()
        .read_json()
        .context("decoding the ollama reply")?;
    match value
        .get("response")
        .and_then(Value::as_str)
    {
        Some(text) => Ok(text.to_string()),
        None => bail!("ollama replied without a `response` field"),
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

/// The first line decides. Anything else is a judge that did not answer the
/// question asked, which is a failure to review and never a reason to block.
pub(super) fn verdict(reply: &str) -> Option<HookOutput> {
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
        "PASS" => None,
        "REVISE"
            if !rest
                .trim()
                .is_empty() =>
        {
            Some(HookOutput::deny(
                "PreToolUse",
                &format!("{DENY_HEAD}\n\n{}", annotate(rest.trim())),
            ))
        }
        // A verdict with nothing to act on is worse than none: it would cost a
        // rewrite with no idea what to change.
        "REVISE" => Some(unreviewed("it objected without naming a rule")),
        _ => Some(unreviewed("it returned no verdict")),
    }
}

fn var(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}
