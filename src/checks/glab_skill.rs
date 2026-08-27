use std::fs;
use std::path::{Path, PathBuf};

use crate::checks::shell;
use crate::input::HookInput;
use crate::output::HookOutput;

/// Traps and API-to-subcommand swaps that the shipped skill does not spell out.
/// Refresh when glab grows a subcommand for something this still sends to `api`.
const TRAPS: &str = include_str!("glab-traps.txt");

const NO_SKILL: &str = "The glab skill is not installed. Install it with \
`glab skills install --path ~/.claude/skills`, or load Skill(\"glab\") if it is \
somewhere else.";

/// Block the first `glab` command per session and hand back the guidance in the
/// denial itself, so the retry is the corrected command rather than a detour
/// through `Skill("glab")`. A per-session marker lets every later call through.
/// Best-effort: if the runtime dir is unavailable the command is allowed
/// (logged on real errors).
pub fn check(input: &HookInput) -> Option<HookOutput> {
    if !is_glab(input.command()) {
        return None;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    decide(
        Path::new(&runtime),
        &input.session_id,
        &reason(load_skill()),
    )
}

/// `~/.claude/skills/glab/SKILL.md`, or the same path under `CLAUDE_CONFIG_DIR`.
fn skill_path() -> Option<PathBuf> {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".claude"),
    };
    Some(base.join("skills/glab/SKILL.md"))
}

/// The skill body, frontmatter dropped — it is discovery metadata, and the model
/// is already past the point of deciding whether this applies.
fn load_skill() -> Option<String> {
    let path = skill_path()?;
    match fs::read_to_string(&path) {
        Ok(text) => Some(strip_frontmatter(&text).to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("glab_skill: read {} failed: {e}", path.display());
            None
        }
    }
}

fn strip_frontmatter(text: &str) -> &str {
    text.strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(text, |(_, body)| body.trim_start())
}

fn reason(skill: Option<String>) -> String {
    format!(
        "{}\n\n{}",
        TRAPS.trim_end(),
        skill
            .as_deref()
            .unwrap_or(NO_SKILL)
    )
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

fn decide(runtime: &Path, session_id: &str, reason: &str) -> Option<HookOutput> {
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
    Some(HookOutput::deny("PreToolUse", reason))
}

#[cfg(test)]
mod tests {
    use super::{NO_SKILL, decide, is_glab, reason, strip_frontmatter};
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

        assert!(
            decide(&base, "sess-1", "r").is_some(),
            "first call must block"
        );
        assert!(
            decide(&base, "sess-1", "r").is_none(),
            "second call must pass"
        );
        assert!(
            decide(&base, "sess-2", "r").is_some(),
            "new session blocks again"
        );

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn frontmatter_goes_but_a_body_without_one_survives() {
        assert_eq!(
            strip_frontmatter("---\nname: glab\n---\n# Title\nbody\n"),
            "# Title\nbody\n"
        );
        assert_eq!(strip_frontmatter("# Title\nbody\n"), "# Title\nbody\n");
        // An unterminated fence is not frontmatter; keep every byte.
        assert_eq!(strip_frontmatter("---\nname: glab\n"), "---\nname: glab\n");
    }

    /// The denial is the only delivery of this guidance, so the traps ride along
    /// whether or not the skill file was found.
    #[test]
    fn traps_ship_with_and_without_the_skill() {
        let with = reason(Some("# GitLab CLI\nskill body".to_string()));
        assert!(with.contains("repo archive` DOWNLOADS"), "{with}");
        assert!(with.contains("skill body"), "{with}");
        assert!(!with.contains(NO_SKILL));

        let without = reason(None);
        assert!(without.contains("repo archive` DOWNLOADS"), "{without}");
        assert!(without.contains(NO_SKILL), "{without}");
    }
}
