use crate::output::HookOutput;

const NO_VERIFY: &str = "`--no-verify` is only allowed for TDD (commit message starts with \"test\"). \
CLAUDE.md forbids skipping git hooks otherwise.";

const NO_SIGN: &str = "Command bypasses git signing (--no-gpg-sign / commit.gpgsign=false). \
CLAUDE.md forbids this unless explicitly requested. If GPG fails on the TTY, run the commit \
manually with `! git commit ...` or fix GPG_TTY.";

pub fn check(command: &str) -> Option<HookOutput> {
    let cmd = command.trim_start();
    if !is_git(cmd) {
        return None;
    }
    if cmd.contains("--no-verify") && !allows_no_verify(cmd) {
        return Some(HookOutput::deny("PreToolUse", NO_VERIFY));
    }
    if cmd.contains("--no-gpg-sign")
        || cmd.contains("commit.gpgsign=false")
        || cmd.contains("commit.gpgsign=0")
    {
        return Some(HookOutput::deny("PreToolUse", NO_SIGN));
    }
    None
}

fn is_git(cmd: &str) -> bool {
    cmd == "git" || cmd.starts_with("git ")
}

/// TDD escape hatch: a commit whose message starts with "test", supplied either
/// inline via `-m` or through a heredoc body.
fn allows_no_verify(cmd: &str) -> bool {
    let mut rest = cmd;
    while let Some(pos) = rest.find("-m") {
        let after = rest[pos + 2..].trim_start();
        let after = after.strip_prefix(['"', '\'']).unwrap_or(after);
        if after.starts_with("test") {
            return true;
        }
        rest = &rest[pos + 2..];
    }
    cmd.contains("<<") && cmd.lines().any(|l| l.trim_start().starts_with("test"))
}

#[cfg(test)]
mod tests {
    use super::check;

    fn blocked(cmd: &str) -> bool {
        check(cmd).is_some()
    }

    #[test]
    fn blocks_bypasses() {
        assert!(blocked("git commit --no-verify -m \"feat: x\""));
        assert!(blocked("git commit --no-gpg-sign -m \"feat: x\""));
        assert!(blocked("git -c commit.gpgsign=false commit -m \"x\""));
        assert!(blocked("git -c commit.gpgsign=0 commit -m \"x\""));
    }

    #[test]
    fn allows_tdd_and_clean() {
        assert!(!blocked("git commit --no-verify -m \"test: red\""));
        assert!(!blocked("git commit --no-verify -m 'test(scope): red'"));
        assert!(!blocked("git commit -m \"feat: x\""));
        assert!(!blocked("git status"));
        assert!(!blocked("cargo test --no-verify-something"));
    }

    #[test]
    fn heredoc_test_message_allowed() {
        let cmd = "git commit --no-verify -F - <<EOF\ntest: red bar\nEOF";
        assert!(!blocked(cmd));
    }
}
