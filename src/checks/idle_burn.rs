use crate::checks::shell;
use crate::output::HookOutput;

const REASON: &str = "A command that only passes time or returns a status (`sleep`, `true`, `false`, `:`) \
does nothing but spend a turn and the tokens of its own tool result. Waiting for a background \
Bash (`run_in_background`) or an agent is not your job: end the turn and say what you are \
waiting for — the notification re-invokes you when it finishes. Waiting on a condition instead \
is the Monitor tool with an until-loop, and pacing a /loop is ScheduleWakeup. A sleep in front \
of real work (`sleep 2 && curl …`) is not this.";

/// Programs whose whole effect is to pass time or hand back an exit status. `wait`
/// is not among them: it blocks on jobs this shell started, which is a real wait.
const IDLERS: [&str; 5] = ["sleep", "usleep", "true", "false", ":"];

pub fn check(command: &str) -> Option<HookOutput> {
    is_idle(command).then(|| HookOutput::deny("PreToolUse", REASON))
}

/// True when every segment either idles or is an `echo` labelling one, and at
/// least one idles. Real work anywhere in the chain — including a poll loop's own
/// condition — takes the whole command out of here.
fn is_idle(command: &str) -> bool {
    let Some(segments) = shell::chain_segments(command) else {
        return false;
    };
    let mut idles = false;
    for segment in &segments {
        if shell::is_lone_echo(segment) {
            continue;
        }
        match shell::program(segment) {
            Some(word) if IDLERS.contains(&word) => idles = true,
            _ => return false,
        }
    }
    idles
}

#[cfg(test)]
mod tests {
    use super::is_idle;

    #[test]
    fn blocks_time_burning() {
        for cmd in [
            "sleep 60",
            "sleep 60; echo waited",
            "sleep 90 && echo done",
            "true",
            ":",
            "false",
            "sleep 5 & sleep 5",
            "/bin/sleep 30",
        ] {
            assert!(is_idle(cmd), "should block: {cmd}");
        }
    }

    #[test]
    fn allows_a_sleep_in_front_of_work() {
        for cmd in [
            "sleep 2 && curl -s localhost:8080/health",
            "while ! nc -z localhost 5432; do sleep 1; done",
            "echo waited",
            "cargo test --release",
            "echo sleep 60",
            "make -j check; sleep 1",
            "wait",
        ] {
            assert!(!is_idle(cmd), "should allow: {cmd}");
        }
    }
}
