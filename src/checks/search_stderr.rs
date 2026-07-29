use crate::checks::shell;
use crate::output::HookOutput;

const SILENCED: &str = "`2>/dev/null` on a search hides the errors worth seeing (wrong path, \
unreadable dir). Drop it, or pass `-s`/`--no-messages`, which suppresses only \
missing/unreadable-file noise and keeps real failures. CLAUDE.md: errors shall not be silenced.";

pub fn check(command: &str) -> Option<HookOutput> {
    shell::chain_segments(command)?
        .iter()
        .any(|segment| shell::is_search(segment) && silences_stderr(segment))
        .then(|| HookOutput::deny("PreToolUse", SILENCED))
}

fn silences_stderr(segment: &str) -> bool {
    let Some(bare) = shell::unquoted(segment) else {
        return false;
    };
    let dense: String = bare
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    ["2>/dev/null", "&>/dev/null", "2>&-", ">/dev/null2>&1"]
        .iter()
        .any(|form| dense.contains(form))
}

#[cfg(test)]
mod tests {
    use super::check;

    fn denied(command: &str) -> bool {
        check(command).is_some()
    }

    #[test]
    fn denies_a_search_that_drops_stderr() {
        for cmd in [
            "grep -rn foo /x 2>/dev/null",
            "grep -rn foo /x 2> /dev/null",
            "rg -n foo /x >/dev/null 2>&1",
            "ls /x; grep -rn foo /y 2>/dev/null",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn leaves_other_commands_alone() {
        for cmd in [
            "grep -rn foo src",
            "grep -rn foo src | head -20",
            // The pattern only *contains* the redirect.
            "grep -rn '2>/dev/null' src",
            "cargo test 2>/dev/null | grep -E '^test result'",
            "ls /x 2>/dev/null",
            // `command grep` opts out of every search rule.
            "command grep -rn foo src 2>/dev/null",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }
}
