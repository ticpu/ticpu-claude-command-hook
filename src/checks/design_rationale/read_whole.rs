//! An edit to a design-rationale.md waits on a whole read of it in this session, and a
//! compaction takes that read back: a summary keeps the headings and loses which
//! section already owns a decision, which is how one gets written twice.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;

use super::{is_rationale, ollama};
use crate::checks::marker;
use crate::input::HookInput;
use crate::output::HookOutput;

const PREFIX: &str = "design-rationale-read-";

const READ_FIRST: &str = "design-rationale.md — read it whole first: the Read tool with no \
offset or limit, once in this session and again after each compaction. Headings and a few \
windows miss the section that already owns this decision. Then re-issue the edit.";

fn name(session: &str, path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{PREFIX}{session}-{:016x}", hasher.finish())
}

/// A `Read` with no window covers the file, the tool's line cap aside.
pub fn record(input: &HookInput) {
    let path = input.file_path();
    let windowed = ["offset", "limit"]
        .iter()
        .any(|key| {
            input
                .tool_input
                .get(key)
                .is_some_and(|value| !value.is_null())
        });
    if input
        .session_id
        .is_empty()
        || !is_rationale(path)
        || windowed
    {
        return;
    }
    let Some(dir) = marker::dir() else {
        return;
    };
    let marker = dir.join(name(&input.session_id, path));
    if let Err(e) = fs::create_dir_all(&dir).and_then(|()| fs::write(&marker, "")) {
        eprintln!(
            "design_rationale: recording the read in {} failed: {e} — the next edit will ask \
             for it again",
            marker.display()
        );
    }
}

/// `None` once this session has read the file whole. Input with no session — a probe, a
/// test — carries nothing to key the read on and is not asked.
pub(super) fn gate(input: &HookInput) -> Option<HookOutput> {
    if input
        .session_id
        .is_empty()
        || marker::present(&name(&input.session_id, input.file_path()))
    {
        return None;
    }
    ollama::warm();
    Some(HookOutput::deny("PreToolUse", READ_FIRST))
}

/// Every read this session recorded, whichever file it was.
pub fn forget(input: &HookInput) {
    let Some(dir) = marker::dir() else {
        return;
    };
    let ours = format!("{PREFIX}{}-", input.session_id);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return,
        Err(e) => {
            eprintln!("design_rationale: listing {} failed: {e}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(&ours)
        {
            continue;
        }
        let path = entry.path();
        if let Err(e) = fs::remove_file(&path) {
            eprintln!("design_rationale: removing {} failed: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(session: &str, tool_input: serde_json::Value) -> HookInput {
        HookInput {
            session_id: session.to_string(),
            tool_input,
            ..HookInput::default()
        }
    }

    const PATH: &str = "/x/docs/design-rationale.md";

    #[test]
    fn a_whole_read_opens_the_gate_and_a_compaction_shuts_it() {
        let session = "read-whole-cycle";
        let edit = input(session, json!({ "file_path": PATH }));
        assert!(gate(&edit).is_some(), "nothing read yet");

        record(&input(session, json!({ "file_path": PATH })));
        assert!(gate(&edit).is_none());

        forget(&input(session, json!({})));
        assert!(gate(&edit).is_some(), "the compaction took the read back");
    }

    #[test]
    fn a_window_is_not_a_whole_read() {
        let session = "read-whole-window";
        record(&input(
            session,
            json!({ "file_path": PATH, "offset": 796, "limit": 24 }),
        ));
        assert!(gate(&input(session, json!({ "file_path": PATH }))).is_some());
    }

    #[test]
    fn a_read_counts_for_its_own_session_and_file_only() {
        record(&input("read-whole-a", json!({ "file_path": PATH })));
        let other_file = "/y/docs/design-rationale.md";
        assert!(gate(&input("read-whole-b", json!({ "file_path": PATH }))).is_some());
        assert!(gate(&input("read-whole-a", json!({ "file_path": other_file }))).is_some());
    }
}
