//! The one way past a judged objection. A model that cannot be overruled turns a
//! wrong finding into a rewrite of correct text, and a model that can overrule
//! itself is not a reviewer at all — so the override is a file the user approves
//! into existence, and it is spent the moment it is used.

use crate::checks::marker;
use crate::output::HookOutput;

/// Named in full on the command line, which is what makes the approval prompt
/// legible: the user is not agreeing to a `touch`, they are agreeing to this.
pub(super) const MARKER: &str = "design-rationale-judge-bypass";

/// A command that would create the marker is prompted whatever the permission
/// rules say, so a `touch` sitting in an allowlist cannot hand one out unseen.
pub fn requested(command: &str) -> Option<HookOutput> {
    marker::creation_requested(command, MARKER).then(|| {
        HookOutput::ask(
            "PreToolUse",
            "This creates a one-shot pass for the next design-rationale.md edit: the judged \
             reviews are skipped for it, and the file is deleted as it is used. Approve only if \
             the objection you were shown is wrong.",
        )
    })
}

pub(super) fn spend() -> bool {
    marker::spend(MARKER)
}

pub(super) fn command() -> String {
    marker::command(MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_command_that_would_create_the_marker_is_prompted() {
        assert!(requested(&command()).is_some());
        assert!(requested(&format!("touch /run/user/1000/claude-hooks/{MARKER}")).is_some());
        assert!(requested(&format!("echo x > \"$XDG_RUNTIME_DIR/{MARKER}\"")).is_some());
        // Buried in a chain behind something innocuous.
        assert!(requested(&format!("ls /tmp && touch ~/.cache/{MARKER}")).is_some());

        assert!(requested("touch /run/user/1000/claude-hooks/glab-skill-abc").is_none());
        assert!(requested("ls").is_none());
    }

    /// Naming the file grants nothing, and a confirmation claiming otherwise is a
    /// lie the user has to see through — this repo's own source mentions it.
    #[test]
    fn reading_or_removing_the_marker_is_not_a_request_for_one() {
        for command in [
            "test -e \"$XDG_RUNTIME_DIR/claude-hooks/design-rationale-judge-bypass\"",
            "rm \"$XDG_RUNTIME_DIR/claude-hooks/design-rationale-judge-bypass\"",
            "ls -l /run/user/1000/claude-hooks/design-rationale-judge-bypass",
            "rg design-rationale-judge-bypass src/",
            "git commit -m \"feat: add design-rationale-judge-bypass\"",
        ] {
            assert!(requested(command).is_none(), "{command}");
        }
    }

    /// The deny quotes this back to the user, so it has to name the file it makes.
    #[test]
    fn the_offered_command_creates_the_marker_it_names() {
        assert!(command().contains(MARKER), "{}", command());
    }
}
