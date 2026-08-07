use std::fs;
use std::path::{Path, PathBuf};

use crate::checks::shell;
use crate::input::HookInput;
use crate::output::HookOutput;

const REASON: &str =
    "Load the glab skill first: Skill(\"glab\"). It has the correct flags and usage \
patterns for the GitLab CLI.";

/// Block the first `glab` command per session to force loading the glab skill;
/// a per-session marker lets every later call through. Best-effort: if the
/// runtime dir is unavailable the command is allowed (logged on real errors).
pub fn check(input: &HookInput) -> Option<HookOutput> {
    if !is_glab(input.command()) {
        return None;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    decide(Path::new(&runtime), &input.session_id)
}

/// True when any segment of the command runs glab, however it is reached — a
/// leading `cd`, a wrapper or an absolute path must not skip the gate.
fn is_glab(command: &str) -> bool {
    match shell::chain_segments(command) {
        Some(segments) => segments
            .iter()
            .any(|segment| {
                shell::pipeline_stages(segment).is_some_and(|stages| {
                    stages
                        .iter()
                        .any(|stage| shell::program(stage) == Some("glab"))
                })
            }),
        // Unanalyzable: fall back to the token the gate originally keyed on rather
        // than let a heredoc carry the first call of the session through.
        None => {
            let cmd = command.trim_start();
            cmd == "glab" || cmd.starts_with("glab ")
        }
    }
}

fn decide(runtime: &Path, session_id: &str) -> Option<HookOutput> {
    let dir = runtime.join("claude-hooks");
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("glab_skill: create_dir_all {} failed: {e}", dir.display());
        return None;
    }
    let marker: PathBuf = dir.join(format!("glab-skill-{session_id}"));
    if marker.exists() {
        return None;
    }
    if let Err(e) = fs::File::create(&marker) {
        eprintln!("glab_skill: create marker {} failed: {e}", marker.display());
        return None;
    }
    Some(HookOutput::deny("PreToolUse", REASON))
}

#[cfg(test)]
mod tests {
    use super::{decide, is_glab};
    use std::path::PathBuf;

    #[test]
    fn matches_glab_only() {
        assert!(is_glab("glab mr view 1"));
        assert!(is_glab("glab"));
        assert!(is_glab("  glab ci"));
        assert!(!is_glab("glabber"));
        assert!(!is_glab("git status"));
    }

    /// The gate exists to force the skill load, so no prefix may skip it.
    #[test]
    fn a_prefix_does_not_skip_the_gate() {
        for cmd in [
            "cd /x && glab mr list",
            "sudo glab mr list",
            "/usr/bin/glab mr list",
            "ls; glab ci status",
            "glab mr list | head",
            "glab mr view 1\nls",
        ] {
            assert!(is_glab(cmd), "{cmd}");
        }
        for cmd in ["cd /x && git status", "echo 'run glab later'"] {
            assert!(!is_glab(cmd), "{cmd}");
        }
    }

    #[test]
    fn first_call_blocks_then_passes() {
        let base: PathBuf = [
            env!("CARGO_MANIFEST_DIR"),
            "target",
            "test-tmp",
            "glab-decide",
        ]
        .iter()
        .collect();
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        assert!(decide(&base, "sess-1").is_some(), "first call must block");
        assert!(decide(&base, "sess-1").is_none(), "second call must pass");
        assert!(
            decide(&base, "sess-2").is_some(),
            "new session blocks again"
        );

        std::fs::remove_dir_all(&base).unwrap();
    }
}
