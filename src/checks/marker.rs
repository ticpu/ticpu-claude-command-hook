//! One-shot approvals: a file the user approves into existence and this binary
//! deletes as it reads it. Each check names its own and words its own prompt; the
//! shape — where it lives, what counts as asking for one — is shared.

use std::fs;
use std::path::{Path, PathBuf};

use crate::checks::shell;

/// Programs that bring a file into existence by naming it. Not a general "writes
/// something" list: the question is only whether this command can end with the
/// marker on disk.
const CREATES: &[&str] = &[
    "touch", "tee", "cp", "mv", "install", "dd", "ln", "truncate",
];

/// True when the command would create this marker. Naming the file is not enough —
/// a `test -e`, an `rm` or a grep of this source mentions it without granting
/// anything, and a confirmation that misdescribes what it is confirming is worse
/// than none.
pub fn creation_requested(command: &str, name: &str) -> bool {
    let Some(segments) = shell::chain_segments(command) else {
        return false;
    };
    segments
        .iter()
        .flat_map(|segment| shell::pipeline_stages(segment).unwrap_or_default())
        .any(|stage| creates_marker(stage, name))
}

/// Either the marker is an argument to something that creates what it names, or
/// it is where the stage's output is being sent.
fn creates_marker(stage: &str, name: &str) -> bool {
    if !stage.contains(name) {
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
                && target.contains(name)
        });
    redirected_to_it || shell::program(stage).is_some_and(|program| CREATES.contains(&program))
}

/// True when the command is that creation and nothing else: one segment, one
/// stage, no redirect, a bare `touch` naming this marker and no other argument.
/// `creation_requested` is looser on purpose — enough to word a prompt, not to
/// grant an allow, which ends the decision for the whole call. `leading_word`
/// rather than `program`: an allow over `sudo touch` is an allow over sudo.
pub fn creation_only(command: &str, name: &str) -> bool {
    let segments = shell::chain_segments(command);
    let Some([segment]) = segments.as_deref() else {
        return false;
    };
    let stages = shell::pipeline_stages(segment);
    let Some([stage]) = stages.as_deref() else {
        return false;
    };
    if shell::redirects_anything(stage) || shell::has_substitution(stage) {
        return false;
    }
    if shell::leading_word(stage) != Some("touch") {
        return false;
    }
    matches!(
        shell::program_args(stage).as_deref(),
        Some([arg]) if names_marker(arg, name)
    )
}

/// The marker as either spelling reaches it: the variable this binary tells the
/// caller to write, or the path that variable expands to here.
fn names_marker(arg: &str, name: &str) -> bool {
    let token = shell::unquote_token(arg);
    token == format!("$XDG_RUNTIME_DIR/claude-hooks/{name}")
        || path(name).is_some_and(|marker| Path::new(token) == marker)
}

/// Where every marker this binary keeps lives. Unit tests get their own, so a run
/// neither reads the switches set on this box nor spends a waiver left for a session.
pub fn dir() -> Option<PathBuf> {
    #[cfg(not(test))]
    return std::env::var_os("XDG_RUNTIME_DIR")
        .map(|runtime| Path::new(&runtime).join("claude-hooks"));
    #[cfg(test)]
    {
        // A path that slips past this seam is refused by the kernel, not spent.
        landlock_test_confine::to_scratch_only(&landlock_test_confine::target_dir());
        Some(landlock_test_confine::scratch_dir(
            "test-scratch/claude-hooks",
        ))
    }
}

fn path(name: &str) -> Option<PathBuf> {
    Some(dir()?.join(name))
}

/// True while the marker exists, leaving it there. The other half of `spend`: a
/// standing switch is read on every decision it governs, a waiver on one.
pub fn present(name: &str) -> bool {
    path(name).is_some_and(|marker| marker.exists())
}

/// True once per file created. Spending it before the decision it overrules means
/// a failure to delete cannot leave a standing waiver behind.
pub fn spend(name: &str) -> bool {
    let Some(marker) = path(name) else {
        return false;
    };
    match fs::remove_file(&marker) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            eprintln!(
                "marker: removing {} failed: {e} — refusing to honour a waiver that would still \
                 be there afterwards",
                marker.display()
            );
            false
        }
    }
}

/// The marker as the user would type it, for a message that has to name the file
/// rather than describe where it lives.
pub fn location(name: &str) -> String {
    format!("\"$XDG_RUNTIME_DIR/claude-hooks/{name}\"")
}

/// The command to hand back, so a refusal can name what to run rather than
/// describe it. The directory is shared with every other marker this binary keeps
/// and is made once per box, so creating it here would be noise on every objection
/// but the first — and a `touch` that fails for want of it says so.
pub fn command(name: &str) -> String {
    format!("touch {}", location(name))
}

#[cfg(test)]
mod tests {
    use super::dir;
    use std::path::Path;

    #[test]
    fn a_thread_reaching_markers_cannot_write_outside_target() {
        dir();
        let stray = Path::new(env!("CARGO_MANIFEST_DIR")).join("landlock-probe");
        let written = std::fs::write(&stray, "");
        if written.is_ok() {
            std::fs::remove_file(&stray).expect("remove the probe");
        }
        assert!(written.is_err(), "wrote {}", stray.display());
    }
}
