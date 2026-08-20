use crate::checks::shell;
use crate::output::HookOutput;

const SILENCED: &str = "`2>/dev/null` on a search hides the errors worth seeing (wrong path, \
unreadable dir). Drop it, or pass `-s`/`--no-messages`, which suppresses only \
missing/unreadable-file noise and keeps real failures. CLAUDE.md: errors shall not be silenced.";

/// Programs whose quoted argument is a command line another shell runs. The
/// approval rule stops at that quote — what the far end chains is its own — but
/// what the transcript is allowed to hide does not.
const SHELL_RUNNERS: [&str; 5] = ["ssh", "sh", "bash", "zsh", "dash"];

/// One `ssh` into another is the depth worth following; past that the body is
/// more likely to be data than a command line.
const MAX_DEPTH: usize = 3;

pub fn check(command: &str) -> Option<HookOutput> {
    silenced_search(command, 0).then(|| HookOutput::deny("PreToolUse", SILENCED))
}

fn silenced_search(command: &str, depth: usize) -> bool {
    let Some(segments) = shell::chain_segments(command) else {
        return false;
    };
    segments
        .iter()
        .any(|segment| {
            (shell::is_search(segment) && silences_stderr(segment))
                || (depth < MAX_DEPTH
                    && shell_bodies(segment)
                        .iter()
                        .any(|body| silenced_search(body, depth + 1)))
        })
}

/// The quoted arguments of every stage that hands one to a shell. A stage that
/// runs no shell contributes none, so a quoted pattern or a document quoting the
/// redirect is never read as a command.
fn shell_bodies(segment: &str) -> Vec<&str> {
    let Some(stages) = shell::pipeline_stages(segment) else {
        return Vec::new();
    };
    stages
        .iter()
        .filter(|stage| shell::command_word(stage).is_some_and(|w| SHELL_RUNNERS.contains(&w)))
        .filter_map(|stage| Some((stage, shell::quoted_spans(stage)?)))
        .flat_map(|(stage, spans)| {
            spans
                .into_iter()
                .map(|span| shell::unquote_token(&stage[span]))
                .collect::<Vec<_>>()
        })
        .collect()
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
            // The body a shell runs at the far end, however it is reached.
            "ssh host 'grep -h ID /var/log/x/*.log 2>/dev/null | head -5'",
            "timeout 30 ssh -o BatchMode=yes host \"grep -rn foo /x 2>/dev/null\"",
            "sudo sh -c 'grep -rn foo /x 2>/dev/null'",
            "ssh a 'ssh b \"grep -rn foo /x 2>/dev/null\"'",
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
            // Only a stage that hands its argument to a shell is descended into.
            "echo 'grep -rn foo /x 2>/dev/null'",
            "ssh host \"echo 'grep -rn foo /x 2>/dev/null'\"",
            "ssh -o 'ProxyCommand=nc %h %p' host uptime",
            "ssh host 'ls /x 2>/dev/null'",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }
}
