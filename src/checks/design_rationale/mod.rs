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
    // The judge sees what the edit introduces, never the text it copies back out of
    // the document to place it: a removal re-emits the section around what it takes
    // out, and prose already in the file draws findings no revision can answer. The
    // countable rules still measure the whole replacement, since a section left over
    // the length bound is over it however much this edit trimmed.
    let introduced = introduced(replaced, added);
    mechanical::check(added).or_else(|| {
        match introduced
            .trim()
            .len()
            >= FLOOR
        {
            false => Some(HookOutput::ask(
                "PreToolUse",
                &format!("{UNJUDGED}{REVIEWED}"),
            )),
            true if bypass::spend() => Some(HookOutput::ask(
                "PreToolUse",
                &format!("{BYPASSED}{REVIEWED}"),
            )),
            true => reviewed(&document, replaced, added, introduced),
        }
    })
}

/// What the edit says that the document did not, which is nothing at all when it only
/// re-wrapped what was there: the file is hard-wrapped, so reflowing a paragraph
/// rewrites every line of it without a word changing, and every rule would then be
/// applied afresh to prose that has already been through them once.
fn introduced<'a>(replaced: &str, added: &'a str) -> &'a str {
    match collapsed(replaced) == collapsed(added) {
        true => "",
        false => new_text(replaced, added),
    }
}

/// What the edit introduces, with the text shared at both ends stripped. An edit
/// appending a section carries an anchor copied out of the document, and one editing
/// a section in place carries whatever it leaves standing around the change.
pub(super) fn new_text<'a>(replaced: &str, added: &'a str) -> &'a str {
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

/// The document is hard-wrapped, so a sentence quoted back as one line is the same
/// sentence as one broken across two.
pub(super) fn collapsed(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A judged objection stops the edit rather than riding along on a permission
/// prompt: an Edit's prompt renders the diff alone, so an objection carried there
/// is one the user never reads before deciding. Stopping puts it in front of the
/// model instead, which can act on it — and the user overrules it with `bypass`.
const OBJECTION: &str = "The design-rationale judge objects. It is a small local model on a \
short rule list, and its usual mistake is reading domain behaviour this project depends on as \
textbook knowledge, so an objection quoting something specific to this codebase is likely wrong.

Revise and re-issue if the objection is right. If it is wrong, do not water the passage down \
until it passes, and do not ask the user whether to overrule: say what the finding is and why \
the passage stands, then run the command below yourself and re-issue the edit unchanged. Its \
permission prompt is where they decide, so putting the same question to them first only spends \
a round trip — and never paste the command for them to run:";

/// Approving the write is the review. Said on every prompt because the writer that
/// asks for one afterwards is asking for a second review of what was just approved,
/// and waits on an answer nobody owes it.
const REVIEWED: &str = " Approving is the review — commit it, and do not ask for another after \
the write.";

const CLEAN: &str = "design-rationale.md — the judge raised nothing. Approve to write it, \
reject to say what should change.";

const BYPASSED: &str = "design-rationale.md — judged review waived for this edit, and the \
waiver is now spent.";

/// A file that does not exist yet is judged with nothing around it, so the rules
/// that ask what a reader of this repo would already know have nothing to read. The
/// objection asks for the frame rather than a narrower passage: cutting the passage
/// is the one repair that cannot work, since the context it is missing is the point.
const NO_DOCUMENT: &str = "\n\nThis file does not exist yet, so the passage was judged with no \
document around it — every rule asking what a reader holding this repo would already know had \
nothing to check against. Before narrowing anything, open the file with the frame it lacks: \
what this component is, what it sits inside, and the boundary the decisions below turn on. Then \
re-issue.";

/// What an objection has to say beyond the finding. Only a first section gets this:
/// once the file has any content, the rules have something to read it against.
fn framing(document: &str) -> &'static str {
    match document
        .trim()
        .is_empty()
    {
        true => NO_DOCUMENT,
        false => "",
    }
}

/// Two reviews of the same edit, run together: one asks what is wrong inside the new
/// text, the other whether the document already says it. A model answers the second
/// only when it is the whole question — beside rules met by quoting a bad passage, a
/// fault that lives in the relation between two sections is never what it reaches for.
///
/// Neither can block on the other's failure, and an objection stands whatever the
/// other review did: a review that did not happen is reported, never assumed to pass.
fn reviewed(document: &str, replaced: &str, added: &str, introduced: &str) -> Option<HookOutput> {
    let (rules, duplication) = std::thread::scope(|scope| {
        let rules = scope.spawn(|| judge::review(document, replaced, introduced));
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
    let framing = framing(document);
    let mut decision = match objections.is_empty() {
        false => HookOutput::deny(
            "PreToolUse",
            &format!(
                "{OBJECTION}\n\n    {}\n\n{}{framing}",
                bypass::command(),
                objections.join("\n")
            ),
        ),
        true => HookOutput::ask("PreToolUse", &format!("{CLEAN}{REVIEWED}")),
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
