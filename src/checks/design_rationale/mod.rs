//! `design-rationale.md` gates. Countable rules are decided here; the rest go to a
//! local model, which catches what it can quote and not what it has to count.

mod judge;
mod mechanical;
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
        .or_else(|| (added.len() >= FLOOR).then(|| judge::check(&document, replaced, added))?)
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
