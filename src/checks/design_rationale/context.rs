//! What the writer answers when the judge says it cannot tell. The rules that decide
//! hardest — is this component ours, would anyone face this decision again — turn on
//! facts about the project that no passage carries, and a model asked to guess at them
//! guesses. So it asks instead, and the answers arrive here.
//!
//! A file rather than the edit itself: the answers are about the passage, not part of
//! it, and writing them into the document to get past the gate is the one repair that
//! must not work. Spent on read like `bypass`, so an answer covers the edit it was
//! written for and not every edit after it.

use std::fs;
use std::path::{Path, PathBuf};

pub(super) const FILE: &str = "design-rationale-judge-context";

fn path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(
        Path::new(&runtime)
            .join("claude-hooks")
            .join(FILE),
    )
}

/// The answers waiting for this edit, taken as they are read. Absent is the normal
/// case — most edits are never asked anything.
pub(super) fn spend() -> String {
    let Some(path) = path() else {
        return String::new();
    };
    let answers = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return String::new(),
        Err(e) => {
            eprintln!("design_rationale: read {} failed: {e}", path.display());
            return String::new();
        }
    };
    if let Err(e) = fs::remove_file(&path) {
        // Kept anyway: the answers are already in hand, and the next edit reading them
        // a second time costs a stale paragraph in one prompt, not a standing waiver.
        eprintln!("design_rationale: removing {} failed: {e}", path.display());
    }
    answers
}

/// Named in the objection so the writer has a path to write rather than a mechanism
/// to infer.
pub(super) fn location() -> String {
    format!("$XDG_RUNTIME_DIR/claude-hooks/{FILE}")
}
