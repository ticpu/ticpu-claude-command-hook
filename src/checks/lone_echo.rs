use crate::checks::shell;
use crate::output::HookOutput;

const ONLY_ECHOES: &str = "prints its own words and nothing else (auto-allowed by the hook)";

/// Allows a command whose every segment is a lone `echo`. `echo "rc=$?"` after a
/// chained command is the whole use of one, and an allowlist entry can only name the
/// wording it was written for, so each new label costs a prompt.
pub fn allow(command: &str) -> Option<HookOutput> {
    let segments = shell::chain_segments(command)?;
    (!segments.is_empty()
        && segments
            .iter()
            .all(|segment| shell::is_lone_echo(segment) && !shell::redirects_anything(segment)))
    .then(|| HookOutput::allow("PreToolUse", ONLY_ECHOES))
}

#[cfg(test)]
mod tests {
    use super::allow;

    fn allowed(command: &str) -> bool {
        allow(command).is_some()
    }

    #[test]
    fn a_status_label_needs_no_prompt() {
        for cmd in [
            "echo $?",
            "echo \"EXIT=$?\"",
            "echo '=== exit ==='",
            "echo start; echo done",
        ] {
            assert!(allowed(cmd), "{cmd}");
        }
    }

    #[test]
    fn anything_that_can_act_keeps_its_prompt() {
        for cmd in [
            // Writes a file, even on the stderr side.
            "echo pwned > /x/f",
            "echo pwned 2> /x/f",
            // Runs something: as a producer, or before echo sees its arguments.
            "echo rm -rf | sh",
            "echo \"$(id)\"",
            "echo `id`",
            // An echo is company here, not the whole command.
            "echo building; cargo test",
        ] {
            assert!(!allowed(cmd), "{cmd}");
        }
    }
}
