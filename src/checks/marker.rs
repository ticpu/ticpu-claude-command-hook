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

fn path(name: &str) -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(
        Path::new(&runtime)
            .join("claude-hooks")
            .join(name),
    )
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

/// The command to hand back, so a refusal can name what to run rather than
/// describe it. The directory is shared with every other marker this binary keeps
/// and is made once per box, so creating it here would be noise on every objection
/// but the first — and a `touch` that fails for want of it says so.
pub fn command(name: &str) -> String {
    format!("touch \"$XDG_RUNTIME_DIR/claude-hooks/{name}\"")
}
