//! The gate every judged passage passes through first: one refusal per draft, asking
//! the writer to take its own text through the authoring rules before anything else
//! reads it.
//!
//! It runs ahead of the model because it is the half that works. A writer holding both
//! the draft and the rules answers better than any verdict extracted from a 12B model —
//! but only when asked, and asked for this draft: a session that read the rules 300k
//! tokens ago is not one that still applies them. So the marker is keyed on the text,
//! not the session. Re-issuing the same passage passes the gate, having been audited;
//! a passage the audit changed is a new draft and is audited once too, which ends when
//! an audit stops changing anything.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::output::HookOutput;

/// One marker per draft, in the directory this binary keeps its others in. They live
/// in `XDG_RUNTIME_DIR`, so the boot clears them and a draft is never audited twice
/// for want of a cleanup nobody wrote.
fn marker(dir: &Path, introduced: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    super::collapsed(introduced).hash(&mut hasher);
    dir.join(format!("design-rationale-audited-{:016x}", hasher.finish()))
}

pub(super) fn gate(introduced: &str) -> Option<HookOutput> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let dir = Path::new(&runtime).join("claude-hooks");
    decide(&dir, introduced)
}

/// `None` once this draft has been asked for. An IO failure asks again rather than
/// waving the draft through: a repeated request costs a round trip, a skipped one
/// costs the pass this gate exists to force.
pub(super) fn decide(dir: &Path, introduced: &str) -> Option<HookOutput> {
    let marker = marker(dir, introduced);
    match marker.try_exists() {
        Ok(true) => return None,
        Ok(false) => {}
        Err(e) => eprintln!("design_rationale: stat {} failed: {e}", marker.display()),
    }
    if let Err(e) = fs::create_dir_all(dir).and_then(|()| fs::write(&marker, "")) {
        // Asked without a marker to show for it, so the next issue of this draft is
        // asked again. Said out loud: a gate that repeats itself silently reads as a
        // model that will not accept anything.
        eprintln!(
            "design_rationale: writing {} failed: {e} — this draft will be asked again",
            marker.display()
        );
    }
    Some(HookOutput::deny("PreToolUse", ASK))
}

const ASK: &str = "design-rationale.md — audit this passage before it lands. Take it through the \
design-rationale clauses in CLAUDE.md one at a time, and answer for each sentence: what future \
change does it inform, and would a competent engineer holding this repo already know it? Cut \
what fails both. A section that survives is usually a third of what was drafted.

Then re-issue the edit — unchanged if the audit changed nothing, and say what you cut if it did. \
This is asked once per draft, so the re-issue goes through to the reviewers behind it. It is not \
a finding and there is nothing here to argue with: no reviewer has read the passage yet.";

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-scratch")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_draft_is_asked_once_and_then_let_through() {
        let dir = scratch("audit-once");
        let draft = "## A decision\n\nThe reader takes the length prefix before it reserves.\n";

        assert!(decide(&dir, draft).is_some(), "the first issue is asked");
        assert!(decide(&dir, draft).is_none(), "the re-issue goes through");

        // A different draft is its own question, however small the change.
        let revised = "## A decision\n\nThe reader takes the length prefix first.\n";
        assert!(decide(&dir, revised).is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    /// The document is hard-wrapped, so a re-issue that only rewrapped the passage is
    /// the same draft and must not be asked again.
    #[test]
    fn a_rewrap_is_the_same_draft() {
        let dir = scratch("audit-rewrap");
        let draft = "## A decision\n\nOne sentence, wrapped as the author left it.\n";
        let rewrapped = "## A decision\n\nOne sentence, wrapped\nas the author left it.\n";

        assert!(decide(&dir, draft).is_some());
        assert!(decide(&dir, rewrapped).is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
