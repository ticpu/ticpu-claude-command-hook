//! `sudo journalctl` is refused outright: reading the journal comes from group
//! membership, so the elevation buys nothing and costs a password prompt this
//! shell cannot answer.

use crate::checks::shell;
use crate::output::HookOutput;

const REASON: &str = "journalctl never requires sudo — reading the system journal comes from \
                      systemd-journal group membership, and root adds nothing to what it prints. \
                      Run the same command without sudo.";

/// How far an `ssh` body is followed. Two hops is a jump host; past that the text
/// being scanned is no longer a command line this can read.
const DEPTH: usize = 2;

pub fn check(command: &str) -> Option<HookOutput> {
    // Judged in front of a heredoc, like the bypass flags: a commit message naming
    // this refusal is prose.
    names_sudo_journalctl(shell::before_heredoc(command), DEPTH)
        .then(|| HookOutput::deny("PreToolUse", REASON))
}

fn names_sudo_journalctl(text: &str, depth: usize) -> bool {
    let segments = shell::chain_segments(text).unwrap_or_else(|| vec![text]);
    segments
        .iter()
        .flat_map(|segment| shell::pipeline_stages(segment).unwrap_or_else(|| vec![segment]))
        .any(|stage| stage_elevates_journalctl(stage, depth))
}

fn stage_elevates_journalctl(stage: &str, depth: usize) -> bool {
    match shell::leading_word(stage) {
        Some("sudo") => shell::program(stage) == Some("journalctl"),
        Some("ssh") if depth > 0 => {
            remote_body(stage).is_some_and(|body| names_sudo_journalctl(&body, depth - 1))
        }
        _ => false,
    }
}

/// The ssh arguments from the first `sudo` onwards, as one command line. Starting
/// there rather than at the destination is what saves an option table: the client's
/// own options are all behind it, and a `sudo` among them is not a spelling ssh has.
fn remote_body(stage: &str) -> Option<String> {
    let args = shell::program_args(stage)?;
    let start = args
        .iter()
        .position(|arg| bare(arg) == "sudo")?;
    let body: Vec<&str> = args[start..]
        .iter()
        .map(|arg| bare(arg))
        .collect();
    Some(body.join(" "))
}

/// Quotes stripped even when the token carries only one end of a span the whole
/// remote command sits in.
fn bare(token: &str) -> &str {
    token.trim_matches(|c| c == '\'' || c == '"')
}

#[cfg(test)]
mod tests {
    use super::check;

    fn denied(command: &str) -> bool {
        check(command).is_some()
    }

    #[test]
    fn sudo_in_front_of_journalctl_is_denied() {
        for cmd in [
            "sudo journalctl -u sshd -n 100",
            "sudo -u root journalctl -xe",
            "/usr/bin/sudo journalctl --vacuum-size=1G",
            "cd /x && sudo journalctl -u foo",
            "sudo journalctl -u foo | grep -i error",
            "ssh srv sudo journalctl -u foo",
            "ssh -o BatchMode=yes srv 'sudo journalctl -u foo'",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn everything_else_passes() {
        for cmd in [
            "journalctl -u sshd -n 100",
            "sudo systemctl restart foo",
            "sudo -u postgres psql",
            "ssh srv journalctl -u foo",
            "git commit -F - <<'EOF'\nfix: refuse sudo journalctl\nEOF",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }
}
