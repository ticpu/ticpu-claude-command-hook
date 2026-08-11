//! `design-rationale.md` gates. Countable rules are decided here; the rest go to a
//! local model, which catches what it can quote and not what it has to count.

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

const SYSTEM: &str = "design-rationale.md edited — stopping for review.";

const CONTEXT: &str = "Edited a design-rationale.md. STOP: wait for the user to review it. No \
further edits, no commit, no proceeding to code until they approve. Do not restate or re-quote \
the section — the tool result already shows it. Say in a line or two which decision it records \
and what standing constraint it adds, then stop. Write only sourced rationale — never fabricate \
a reason.";

/// Below this the edit carries no prose to judge: a deletion, a link fix, a heading
/// rename. Measured against real edits, which cluster well under it or well over.
const FLOOR: usize = 200;

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
    mechanical::check(added)
        .or_else(|| (added.len() >= FLOOR).then(|| reviewed(&document, replaced, added))?)
}

const DENY_HEAD: &str = "The design-rationale judge objects. Revise the section and re-issue \
the edit — or say why the objection is wrong and I will pass it on.";

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
    match objections.is_empty() {
        false => {
            let mut denial = HookOutput::deny(
                "PreToolUse",
                &format!("{DENY_HEAD}\n\n{}", objections.join("\n")),
            );
            denial.system_message = unreviewed;
            Some(denial)
        }
        // Never a deny: an edit is not blocked because a reviewer could not be
        // reached. Not a silent pass either — this line is the only sign that
        // nothing was read.
        true => unreviewed.map(|why| HookOutput::note(&why)),
    }
}

pub fn post_tool_use(file_path: &str) -> Option<HookOutput> {
    is_rationale(file_path).then(|| HookOutput::context(SYSTEM, CONTEXT))
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
