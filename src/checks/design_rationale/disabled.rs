//! The gate turned off for a while. A waiver answers one wrong finding; a session
//! rewriting the file section by section is a different thing, and spending a round
//! trip per draft to say so is the cost this removes. So this one is not spent: it
//! stands until it is removed, and every edit it lets through says it was unread.

use crate::checks::marker;
use crate::output::HookOutput;

pub const MARKER: &str = "design-rationale-gate-off";

/// Read, never consumed. The directory it sits in does not survive a logout, which
/// is the only bound on how long "temporarily" lasts.
pub(super) fn active() -> bool {
    marker::present(MARKER)
}

/// Said on the permission prompt of every edit made under it, rather than passing
/// the edit through silently: a switch nobody is reminded of is one left on.
pub(super) fn notice() -> HookOutput {
    HookOutput::ask(
        "PreToolUse",
        &format!(
            "design-rationale.md — the gate is off, so nothing has read this edit: not the \
             countable rules, not the judge. Restore it with:\n  rm {}",
            marker::location(MARKER)
        ),
    )
}

/// The writer is told the same thing after the fact, since the prompt above is
/// addressed to whoever answers it. Without this the edit reads as reviewed —
/// which is what an approved prompt on this file normally means.
pub(super) const UNREVIEWED: &str = "The design-rationale gate is off: the edit you just made \
was reviewed by nothing. Approving the prompt was permission, not a verdict. Hold the passage to \
the CLAUDE.md authoring clauses yourself.";

/// Creating it is prompted whatever the permission rules say. This one is not spent
/// on use, so an allowlisted `touch` would otherwise take the gate off for the rest
/// of the session with nobody told.
pub fn requested(command: &str) -> Option<HookOutput> {
    marker::creation_requested(command, MARKER).then(|| {
        HookOutput::ask(
            "PreToolUse",
            "This turns the design-rationale.md review gate off and leaves it off — every edit \
             until the file is removed goes unreviewed. It is not spent on use, and it lives \
             until logout.",
        )
    })
}
