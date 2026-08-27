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
    let mut refused = HookOutput::deny("PreToolUse", ASK.trim_end());
    // The deny is addressed to the writer, and a refused edit renders no diff, so this
    // is the only place the draft is visible to the person the audit is done for.
    refused.system_message = Some(format!(
        "design-rationale.md — this draft goes back to its author to audit:\n\n{}",
        excerpt(introduced)
    ));
    Some(refused)
}

/// Long enough for a padded section to be read as one, and bounded so an edit adding
/// several does not take the terminal with it.
const EXCERPT: usize = 1600;

fn excerpt(introduced: &str) -> String {
    let draft = introduced.trim();
    match draft
        .char_indices()
        .nth(EXCERPT)
    {
        None => draft.to_string(),
        Some((cut, _)) => format!("{}…", &draft[..cut]),
    }
}

/// Emitted on every draft, so it is kept where `/compress-messages` can rewrite it
/// without touching the code that sends it. The trailing newline is the file's, not
/// the message's.
const ASK: &str = include_str!("audit-ask.txt");

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

    /// A refused edit renders no diff, so the draft has to reach the screen some other
    /// way or the audit happens where its author cannot see it.
    #[test]
    fn the_refusal_shows_the_draft() {
        let dir = scratch("audit-shows");
        let draft = "## A decision\n\nThe sentence the author has to be able to read back.\n";

        let shown = decide(&dir, draft)
            .expect("asked")
            .system_message
            .expect("the draft is shown");
        assert!(shown.contains("The sentence the author has to be able to read back."));

        // Bounded, and never cut inside a character.
        let long = "é".repeat(EXCERPT * 2);
        let shown = decide(&dir, &long)
            .expect("asked")
            .system_message
            .expect("the draft is shown");
        assert!(shown.ends_with('…'), "{shown}");

        let _ = fs::remove_dir_all(&dir);
    }

    /// The ask reaches its writer only by refusing: an edit prompted rather than
    /// refused renders its diff to the user, and the pass nobody was asked for is the
    /// one that would have cut the passage. The instruction travels on the decision,
    /// where the writer reads it; the draft travels beside it, the writer holding its
    /// own already.
    #[test]
    fn the_ask_refuses_and_carries_the_passes_to_make() {
        let dir = scratch("audit-decision");
        let draft = "## A decision\n\nOne sentence, and the audit it has not had yet.\n";

        let decision = decide(&dir, draft)
            .expect("asked")
            .hook_specific_output
            .expect("a decision");
        assert_eq!(
            decision
                .permission_decision
                .as_deref(),
            Some("deny")
        );
        let reason = decision
            .permission_decision_reason
            .expect("a deny carries a reason");
        assert!(!reason.contains(draft), "{reason}");
        assert!(reason.contains("re-issue"), "{reason}");

        let _ = fs::remove_dir_all(&dir);
    }

    /// Each pass is here because a draft cleared the ones before it: a passage that
    /// belonged at the call site it described, a decision a convention in the repo
    /// already settled, and a consequence that took a paragraph of its own.
    #[test]
    fn the_ask_makes_every_pass() {
        for pass in [
            "Whole first",
            "already owns the decision",
            "paragraph by paragraph",
            "sentence by sentence",
        ] {
            assert!(ASK.contains(pass), "{pass}");
        }
    }

    /// One marker per draft, so a draft written while another is in flight neither
    /// spends its audit nor is spent by it.
    #[test]
    fn two_drafts_in_flight_keep_their_own_audits() {
        let dir = scratch("audit-interleaved");
        let first = "## A decision\n\nThe reader takes the length prefix before it reserves.\n";
        let second = "## Another\n\nThe writer holds the lock across the flush, never past it.\n";

        assert!(decide(&dir, first).is_some());
        assert!(decide(&dir, second).is_some());
        assert!(decide(&dir, first).is_none(), "the first was audited");
        assert!(decide(&dir, second).is_none(), "so was the second");

        let _ = fs::remove_dir_all(&dir);
    }

    /// A marker that cannot be written leaves the draft unrecorded, and the next issue
    /// of it is asked again rather than waved through: a repeated ask costs a round
    /// trip, a skipped one costs the pass this gate exists to force.
    #[test]
    fn a_marker_that_cannot_be_written_asks_again() {
        let dir = scratch("audit-unwritable");
        fs::create_dir_all(
            dir.parent()
                .expect("under the scratch root"),
        )
        .expect("scratch root");
        // A file where the marker directory would go: neither the stat nor the write
        // beneath it can succeed.
        fs::write(&dir, "").expect("the blocking file");

        let draft = "## A decision\n\nAsked once, and once more for want of a marker.\n";
        assert!(decide(&dir, draft).is_some());
        assert!(decide(&dir, draft).is_some(), "asked again");

        let _ = fs::remove_file(&dir);
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
