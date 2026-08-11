//! `design-rationale.md` gates. Countable rules are decided here; the rest go to a
//! local model, which catches what it can quote and not what it has to count.

pub mod bypass;
mod judge;
mod mechanical;
mod ollama;
mod overlap;
#[cfg(test)]
mod tests;

use std::fs;
use std::io::ErrorKind;

use crate::input::HookInput;
use crate::output::HookOutput;

/// Below this the edit carries no prose to judge: a deletion, a link fix, a heading
/// rename. Measured against real edits, which cluster well under it or well over.
const FLOOR: usize = 200;

const UNJUDGED: &str = "design-rationale.md — too small to judge, so nobody read it but you.";

pub fn pre_tool_use(input: &HookInput) -> Option<HookOutput> {
    let path = input.file_path();
    if !is_rationale(path) {
        return None;
    }
    let document = read_document(path);
    // A `Write` replaces the whole file, so what it takes out is what is on disk.
    let (replaced, added) = match input
        .tool_name
        .as_str()
    {
        "Write" => (document.as_str(), input.content()),
        _ => (input.old_string(), input.new_string()),
    };
    // Every edit to the file stops for the user, judged or not: approving is the
    // review, so an edit that slipped past on size would be one nobody saw. The
    // countable rules are not covered by the bypass — a heading form is not a
    // finding anyone needs to overrule.
    mechanical::check(added).or_else(|| match added.len() >= FLOOR {
        false => Some(HookOutput::ask("PreToolUse", UNJUDGED)),
        true if bypass::spend() => Some(HookOutput::ask("PreToolUse", BYPASSED)),
        true => reviewed(&document, replaced, added),
    })
}

/// A judged objection stops the edit rather than riding along on a permission
/// prompt: an Edit's prompt renders the diff alone, so an objection carried there
/// is one the user never reads before deciding. Stopping puts it in front of the
/// model instead, which can act on it — and the user overrules it with `bypass`.
const OBJECTION: &str = "The design-rationale judge objects. It is a small local model on a \
short rule list, and its usual mistake is reading domain behaviour this project depends on as \
textbook knowledge, so an objection quoting something specific to this codebase is likely wrong.

Revise and re-issue if the objection is right. If it is wrong, say so to the user with the \
finding and why the passage stands, rather than watering the passage down until it passes. They \
can wave it through with:";

const CLEAN: &str = "design-rationale.md — the judge raised nothing. Approve to write it, \
reject to say what should change.";

const BYPASSED: &str = "design-rationale.md — judged review waived for this edit, and the \
waiver is now spent.";

/// Two reviews of the same edit, run together: one asks what is wrong inside the new
/// text, the other whether the document already says it. A model answers the second
/// only when it is the whole question — beside rules met by quoting a bad passage, a
/// fault that lives in the relation between two sections is never what it reaches for.
///
/// Neither can block on the other's failure, and an objection stands whatever the
/// other review did: a review that did not happen is reported, never assumed to pass.
fn reviewed(document: &str, replaced: &str, added: &str) -> Option<HookOutput> {
    let (rules, duplication) = std::thread::scope(|scope| {
        let rules = scope.spawn(|| judge::review(document, replaced, added));
        let duplication = overlap::review(document, replaced, added);
        (rules.join(), duplication)
    });

    let mut objections = Vec::new();
    let mut failures = Vec::new();
    let mut collect =
        |what: &str, outcome: Result<anyhow::Result<Option<String>>, String>| match outcome {
            Ok(Ok(Some(objection))) => objections.push(objection),
            Ok(Ok(None)) => {}
            Ok(Err(e)) => failures.push(format!("{what}: {e:#}")),
            Err(e) => failures.push(format!("{what}: {e}")),
        };
    collect(
        "rules",
        rules.map_err(|_| "the review panicked".to_string()),
    );
    collect("duplication", Ok(duplication));

    let unreviewed = (!failures.is_empty()).then(|| {
        format!(
            "design-rationale judge did not run: {}",
            failures.join("; ")
        )
    });
    let mut decision = match objections.is_empty() {
        false => HookOutput::deny(
            "PreToolUse",
            &format!(
                "{OBJECTION}\n\n    {}\n\n{}",
                bypass::command(),
                objections.join("\n")
            ),
        ),
        true => HookOutput::ask("PreToolUse", CLEAN),
    };
    // A review that did not happen is said out loud rather than assumed to pass,
    // and never blocks: the edit is decided on whatever the reviewers managed.
    decision.system_message = unreviewed;
    Some(decision)
}

fn is_rationale(file_path: &str) -> bool {
    file_path.ends_with("design-rationale.md")
}

/// The file the edit lands in. Absent is normal — a `Write` creating it — but any
/// other read failure is reported rather than passed off as an empty document,
/// since the judge would then miss every duplicate.
fn read_document(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => {
            eprintln!("design_rationale: read {path} failed: {e}");
            String::new()
        }
    }
}
