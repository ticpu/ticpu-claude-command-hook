//! The one way past a judged objection. A model that cannot be overruled turns a
//! wrong finding into a rewrite of correct text, and a model that can overrule
//! itself is not a reviewer at all — so the override is a file the user approves
//! into existence, and it is spent the moment it is used.

use std::fs;
use std::path::{Path, PathBuf};

use crate::checks::shell;
use crate::output::HookOutput;

/// Named in full on the command line, which is what makes the approval prompt
/// legible: the user is not agreeing to a `touch`, they are agreeing to this.
pub(super) const MARKER: &str = "design-rationale-judge-bypass";

/// Programs that bring a file into existence by naming it. Not a general "writes
/// something" list: the question is only whether this command can end with the
/// marker on disk.
const CREATES: &[&str] = &[
    "touch", "tee", "cp", "mv", "install", "dd", "ln", "truncate",
];

/// A command that would create the marker is prompted whatever the permission
/// rules say, so a `touch` sitting in an allowlist cannot hand one out unseen.
/// Naming the file is not enough — a `test -e`, an `rm` or a grep of this source
/// mentions it without granting anything, and a confirmation that misdescribes
/// what it is confirming is worse than none.
pub fn requested(command: &str) -> Option<HookOutput> {
    let creates = shell::chain_segments(command)?
        .iter()
        .flat_map(|segment| shell::pipeline_stages(segment).unwrap_or_default())
        .any(creates_marker);
    creates.then(|| {
        HookOutput::ask(
            "PreToolUse",
            "This creates a one-shot pass for the next design-rationale.md edit: the judged \
             reviews are skipped for it, and the file is deleted as it is used. Approve only if \
             the objection you were shown is wrong.",
        )
    })
}

/// Either the marker is an argument to something that creates what it names, or
/// it is where the stage's output is being sent.
fn creates_marker(stage: &str) -> bool {
    if !stage.contains(MARKER) {
        return false;
    }
    let redirected_to_it = stage
        .split('>')
        .skip(1)
        .any(|target| {
            target
                .trim_start_matches('>')
                .trim_start()
                .starts_with(['"', '\'', '$', '/'])
                && target.contains(MARKER)
        });
    redirected_to_it || shell::program(stage).is_some_and(|program| CREATES.contains(&program))
}

fn marker() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(
        Path::new(&runtime)
            .join("claude-hooks")
            .join(MARKER),
    )
}

/// True once per file created. Spending it before the edit is judged rather than
/// after means a failure to delete cannot leave a standing bypass behind.
pub(super) fn spend() -> bool {
    let Some(marker) = marker() else {
        return false;
    };
    match fs::remove_file(&marker) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            eprintln!(
                "design_rationale: removing {} failed: {e} — refusing to honour a bypass that \
                 would still be there afterwards",
                marker.display()
            );
            false
        }
    }
}

/// The command to hand back, so the deny can name what to run rather than
/// describe it. The directory is shared with every other marker this binary
/// keeps and is made once per box, so creating it here would be noise on every
/// objection but the first — and a `touch` that fails for want of it says so.
pub(super) fn command() -> String {
    format!("touch \"$XDG_RUNTIME_DIR/claude-hooks/{MARKER}\"")
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
