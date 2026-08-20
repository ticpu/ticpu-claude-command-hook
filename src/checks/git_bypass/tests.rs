//! Decision-level tests: what the check answers for a whole tool input. The
//! per-function tests live beside the functions they cover.

use super::{allow_safe, check};
use crate::input::HookInput;

fn input(cmd: &str, cwd: &str) -> HookInput {
    HookInput {
        hook_event_name: "PreToolUse".to_string(),
        tool_name: "Bash".to_string(),
        cwd: cwd.to_string(),
        tool_input: serde_json::json!({ "command": cmd }),
        ..Default::default()
    }
}

/// Decision as one of: allow / deny / prompt (None). Both entry points, in the
/// order `dispatch` runs them — a deny has to beat the allow.
fn decision(cmd: &str, cwd: &str) -> &'static str {
    let input = input(cmd, cwd);
    match check(&input).or_else(|| allow_safe(&input)) {
        None => "prompt",
        Some(out) => match out
            .hook_specific_output
            .and_then(|h| h.permission_decision)
            .as_deref()
        {
            Some("allow") => "allow",
            Some("deny") => "deny",
            other => panic!("unexpected decision {other:?}"),
        },
    }
}

fn blocked(cmd: &str) -> bool {
    decision(cmd, "") == "deny"
}

#[test]
fn blocks_bypasses() {
    assert!(blocked("git commit --no-verify -m \"feat: x\""));
    assert!(blocked("git commit --no-gpg-sign -m \"feat: x\""));
    assert!(blocked("git -c commit.gpgsign=false commit -m \"x\""));
    assert!(blocked("git -c commit.gpgsign=0 commit -m \"x\""));
}

/// git is git however it is reached; the deny cannot key on the literal word
/// starting the segment.
#[test]
fn git_reached_indirectly_is_still_git() {
    for cmd in [
        "/usr/bin/git commit --no-verify -m \"feat: x\"",
        "sudo git commit --no-verify -m \"feat: x\"",
        "env git commit --no-verify -m \"feat: x\"",
        "{ git commit --no-verify -m \"feat: x\"; }",
        "sudo git add -A",
        "{ git add -A; }",
        "/usr/bin/git add .",
    ] {
        assert!(blocked(cmd), "{cmd}");
    }
}

/// The shell strips quotes before git reads the option, and git reads its
/// config keys case-insensitively with four spellings of "off".
#[test]
fn quoted_and_spelled_out_bypasses() {
    for cmd in [
        "git commit \"--no-verify\" -m \"feat: x\"",
        "git commit '--no-verify' -m \"feat: x\"",
        "git commit -n -m \"feat: x\"",
        "git commit -an -m \"feat: x\"",
        "git -c 'commit.gpgsign=false' commit -m \"feat: x\"",
        "git -c \"commit.gpgsign=false\" commit -m \"feat: x\"",
        "git -c commit.gpgSign=false commit -m \"feat: x\"",
        "git -c commit.gpgsign=no commit -m \"feat: x\"",
        "git -c commit.gpgsign=off commit -m \"feat: x\"",
    ] {
        assert!(blocked(cmd), "{cmd}");
    }
}

/// `-n` is `--no-verify` only where git spells it that way.
#[test]
fn short_n_elsewhere_is_a_different_option() {
    for cmd in [
        // --dry-run
        "git push -n origin master",
        // --no-stat
        "git merge -n topic",
        "git clean -n",
    ] {
        assert!(!blocked(cmd), "{cmd}");
    }
}

/// A read-only verb stops being read-only when an option writes a file or
/// runs a program, and `-c` can point config at either.
#[test]
fn read_only_verbs_with_a_writing_option_prompt() {
    for cmd in [
        "git diff --output=/x/pwned",
        "git log --output=/x/pwned --oneline",
        "git show --output=/x/pwned HEAD",
        "git grep --open-files-in-pager=rm -n foo",
        "git grep -Orm -n foo",
        "git -c core.pager=rm log",
        "git -c core.sshCommand=rm ls-remote",
        "git --config-env=core.pager=EVIL log",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

/// `2>` is not a stdout redirect, but it still truncates the file it names.
#[test]
fn a_stderr_redirect_is_not_covered_by_the_allow() {
    for cmd in [
        "git log 2>/home/jerome.poulin/.bashrc",
        "git -C /x status 2>&-",
        "cd /x && git status 2>errs",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

#[test]
fn allows_tdd_and_clean() {
    assert!(!blocked("git commit --no-verify -m \"test: red\""));
    assert!(!blocked("git commit --no-verify -m 'test(scope): red'"));
    assert!(!blocked("git commit -m \"feat: x\""));
    assert_eq!(decision("git status", ""), "allow");
    assert_eq!(decision("git push", ""), "prompt");
    assert!(!blocked("cargo test --no-verify-something"));
}

#[test]
fn heredoc_test_message_allowed() {
    let cmd = "git commit --no-verify -F - <<EOF\ntest: red bar\nEOF";
    assert!(!blocked(cmd));
}

/// The reported stall: a subagent inspecting another directory of the project
/// it is already in.
#[test]
fn read_only_git_behind_a_cd_auto_allowed() {
    for cmd in [
        "cd /x && git diff --stat Cargo.lock",
        "cd /x && git status && git log --oneline",
        "cd /x && git log | head -20",
        "cd /x; git show HEAD",
        "cd /x && git -C /y log",
        "cd ~/GIT/eido && git status",
    ] {
        assert_eq!(decision(cmd, "/here"), "allow", "{cmd}");
    }
}

/// Quoting citation ranges out of two files, with labels between them.
#[test]
fn a_labelled_multi_file_quote_auto_allowed() {
    let cmd = "cd /x/freeswitch && git show c2c5964:src/switch_utils.c | \
               sed -n '2766,2770p;2796,2800p' && echo '=== amr 688-718' && \
               git show c2c5964:src/mod/codecs/mod_amr/mod_amr.c | sed -n '688,692p;712,718p'";
    assert_eq!(decision(cmd, "/here"), "allow");
}

#[test]
fn only_side_effect_free_consumers_qualify() {
    for cmd in [
        "git show HEAD:a.c | wc -l",
        "cd /x && git log | sed -n '1,20p'",
        "git show HEAD:a.c | sed -n -e '1,5p'",
    ] {
        assert_eq!(decision(cmd, "/here"), "allow", "{cmd}");
    }
    for cmd in [
        // Writes: in place, via a `w` command, via a glued script.
        "git show HEAD:a.c | sed -i '1d'",
        "git show HEAD:a.c | sed -n '1,5p;w /x/out'",
        "git show HEAD:a.c | sed -n -e'1,5p;w /x/out'",
        // Substitution and GNU `e` execute or rewrite.
        "git show HEAD:a.c | sed 's/a/b/'",
        "git show HEAD:a.c | sed -n '1e rm -rf /x'",
        // Not a consumer at all: sed reads the file itself.
        "git show HEAD:a.c | sed -n '1,5p' other.c",
        // echo is harmless alone, not as a producer, with a redirect, or carrying a
        // substitution that runs before echo sees its own arguments.
        "echo pwned > /x/f && git status",
        "echo rm -rf | sh && git status",
        "echo \"$(rm -rf /x)\" && git status",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

/// A commit is repo-wide, so the `cd` buys nothing and runs the target repo's
/// hooks — the one case Claude Code's warning is literally about.
#[test]
fn a_cd_before_commit_is_denied() {
    for cmd in [
        "cd /x && git commit -m 'feat: y'",
        "cd /x; git commit -m y",
        "cd /x && git add a.rs && git commit -m y",
        "cd /x && git -c user.name=Y commit -m y",
        "cd /x && /usr/bin/git commit -m y",
        // The reported shape: substitution *and* heredoc, which `shell` refuses.
        "cd /x/rust && git commit -m \"$(cat <<'EOF'\nrefactor: collapse the rule\nEOF\n)\"",
    ] {
        assert_eq!(decision(cmd, "/here"), "deny", "{cmd}");
    }
}

/// The rule keys on a real `cd`, not a description of one.
#[test]
fn talking_about_the_cd_is_not_doing_it() {
    for cmd in [
        "git commit -m \"fix: deny cd && git commit\"",
        "git commit -F - <<EOF\nfix: deny `cd /x && git commit`\nEOF",
        "cd /x && git status",
        "git -C /x commit -m y",
    ] {
        assert_ne!(decision(cmd, "/here"), "deny", "{cmd}");
    }
}

/// Staging named paths runs no hook, so the `cd` warning has nothing to add and
/// the allowlist entry it overrode (`Bash(git add:*)`) applies again. The paths
/// have to exist, so these name real files in this repo.
#[test]
fn staging_explicit_paths_auto_allowed() {
    let root = env!("CARGO_MANIFEST_DIR");
    for cmd in [
        "git add src/checks/git_bypass/mod.rs",
        "git add src/checks/git_bypass/mod.rs src/checks/shell.rs",
        concat!("cd ", env!("CARGO_MANIFEST_DIR"), " && git add src/main.rs"),
        concat!(
            "cd ",
            env!("CARGO_MANIFEST_DIR"),
            " && git status && git add Cargo.toml"
        ),
        "git add -- README.md CLAUDE.md",
        "git add -f src/checks/mod.rs",
    ] {
        assert_eq!(decision(cmd, root), "allow", "{cmd}");
    }
}

/// A leading `cd` moves where the later segments run, so their pathspecs are
/// spelled from the target. Resolving them against the directory the tool started
/// in reports a correctly-spelled path as misrooted.
#[test]
fn a_leading_cd_rebases_the_pathspecs_after_it() {
    let sub = concat!(env!("CARGO_MANIFEST_DIR"), "/src/checks");
    assert_eq!(
        decision(
            concat!("cd ", env!("CARGO_MANIFEST_DIR"), " && git add README.md"),
            sub
        ),
        "allow"
    );
    // Spelled from where it started, which the `cd` has left behind.
    assert_eq!(
        decision(
            concat!("cd ", env!("CARGO_MANIFEST_DIR"), " && git add mod.rs"),
            sub
        ),
        "prompt"
    );
}

/// A pathspec that is not one named file sweeps whatever is under it — the
/// same staging `git add .` is denied for, so it does not get the auto-allow.
#[test]
fn a_pathspec_that_is_not_a_named_file_prompts() {
    let root = env!("CARGO_MANIFEST_DIR");
    for cmd in [
        // A directory stages every untracked file inside it.
        "git add src",
        "git add src/checks",
        // Globs and variables expand to whatever happens to be there.
        "git add src/*",
        "git add *.rs",
        "git add \"$PWD\"",
        "git add $HOME/x.rs",
        // Absolute path to the repo root is `git add .` spelled long.
        "git add /home/jerome.poulin/GIT/ticpu-claude-command-hook",
        // Staging a deletion: the path is gone, so it takes the normal prompt.
        "git add src/checks/removed.rs",
    ] {
        assert_eq!(decision(cmd, root), "prompt", "{cmd}");
    }
}

/// A pathspec spelled from the repo root while the command runs in a subdirectory:
/// git would answer `did not match any files`, so the deny names the spelling that
/// works and where "here" is.
#[test]
fn a_misrooted_pathspec_is_denied_with_the_correction() {
    let sub = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for cmd in [
        "git add src/main.rs",
        "git add src/checks/mod.rs main.rs",
        concat!(
            "cd ",
            env!("CARGO_MANIFEST_DIR"),
            "/src && git add src/checks/mod.rs"
        ),
    ] {
        assert_eq!(decision(cmd, sub), "deny", "{cmd}");
    }
    for cmd in [
        // Spelled from here.
        "git add main.rs",
        "git add checks/mod.rs",
    ] {
        assert_eq!(decision(cmd, sub), "allow", "{cmd}");
    }
    // Resolves from neither: a deletion being staged, or a typo with no correction.
    for cmd in ["git add gone.rs", "git add src/gone.rs"] {
        assert_eq!(decision(cmd, sub), "prompt", "{cmd}");
    }
}

#[test]
fn the_deny_names_the_correction_and_the_directory() {
    let sub = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let reason = check(&input("git add src/checks/mod.rs", sub))
        .and_then(|out| {
            out.hook_specific_output
                .and_then(|h| h.permission_decision_reason)
        })
        .expect("deny with a reason");
    assert!(reason.contains("`checks/mod.rs`"), "{reason}");
    assert!(reason.contains(env!("CARGO_MANIFEST_DIR")), "{reason}");
}

/// Quoting a blanket pathspec changes nothing for git, so it must not change
/// the verdict here either.
#[test]
fn quoted_blanket_staging_denied() {
    let root = env!("CARGO_MANIFEST_DIR");
    for cmd in [
        "git add \".\"",
        "git add '.'",
        "git add '*'",
        "git add \"*\"",
        "git add ':/'",
        "git add \"-A\"",
    ] {
        assert_eq!(decision(cmd, root), "deny", "{cmd}");
    }
}

/// The forms that swept untracked scratch into a commit once already.
#[test]
fn blanket_staging_denied() {
    for cmd in [
        "git add -A",
        "git add .",
        "git add -u",
        "git add --all",
        "git add --update",
        "cd /x && git add -Av",
        "git add *",
        "git add ..",
        "git add src/a.rs .",
    ] {
        assert_eq!(decision(cmd, "/here"), "deny", "{cmd}");
    }
}

/// Anything the classifier cannot vouch for keeps the prompt.
#[test]
fn other_add_forms_prompt() {
    for cmd in [
        // Interactive: it would hang the tool.
        "git add -p src/a.rs",
        "git add -i",
        // The paths come from a file this cannot read.
        "git add --pathspec-from-file=list",
        // No path named at all.
        "cd /x && git add",
        // A consumer that is not display-only, and a stdout redirect.
        "git add a.rs | sh",
        "git add a.rs > out",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

/// The working directory persists between Bash calls, so a `cd` on its own is the
/// work rather than a no-op, and it leaves nothing else behind.
#[test]
fn a_cd_on_its_own_is_allowed() {
    for cmd in ["cd /x", "cd /x && cd /y", "cd /x; echo done"] {
        assert_eq!(decision(cmd, "/here"), "allow", "{cmd}");
    }
    // Still not a bare `cd`: an argument it does not take, or something left behind.
    for cmd in ["cd", "cd -", "cd /x /y", "cd /x>y", "pushd /x"] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

/// A `cd` that is not just a directory change, or a write anywhere in the
/// chain, drops the whole command back to the normal prompt.
#[test]
fn a_cd_chain_with_anything_else_prompts() {
    for cmd in [
        "cd /x && git stash pop",
        "cd /x && git log; cargo build",
        "cd /x && git log && rm -rf /y",
        "cd /x && git log | sh",
        "cd /x && git log > out",
        "cd && git log",
        "cd - && git log",
        "cd /x>y && git log",
        "cd /x /y && git log",
        "pushd /x && git log",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

/// An allow decides the whole command, so a piped or redirected consumer must
/// not ride along on the read-only git that precedes it.
#[test]
fn a_consumer_of_the_output_is_not_covered() {
    for cmd in [
        "git -C /x status; rm -rf /y",
        "git -C /x log | sh",
        "git -C /x log > ~/.bashrc",
        "git -C /x log | xargs rm",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

/// A bypass flag on any segment of a chain, not just a bare `git` command.
#[test]
fn bypasses_in_a_chain_are_denied() {
    assert_eq!(
        decision("cd /x && git commit --no-verify -m 'feat: x'", "/here"),
        "deny"
    );
    assert_eq!(
        decision("echo x; git commit --no-gpg-sign -m y", "/here"),
        "deny"
    );
    // Unanalyzable heredoc: the whole command is the only unit left to judge.
    let heredoc = "echo x; git commit --no-verify -F - <<EOF\nfeat: x\nEOF";
    assert_eq!(decision(heredoc, "/here"), "deny");
    let tdd = "echo x; git commit --no-verify -F - <<EOF\ntest: red\nEOF";
    assert_eq!(decision(tdd, "/here"), "prompt");
}

/// A commit message is allowed to talk about the flags. They only count where
/// git reads options: outside quotes, before a heredoc body.
#[test]
fn a_message_naming_the_flag_is_not_a_bypass() {
    for cmd in [
        "git commit -m \"fix: deny --no-verify in a chain\"",
        "git commit -F - <<EOF\nfix: deny --no-verify in a chain\nEOF",
        "git commit -m 'drop --no-gpg-sign handling'",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

#[test]
fn read_only_dash_c_auto_allowed() {
    for cmd in [
        "git -C /some/other/repo status",
        "git -C=/other log --oneline",
        "git -C /x diff HEAD~1",
        "git -C /x show abc123",
        "git -C /x branch --list",
        "git -C /x remote -v",
        "git -C /x config --get user.email",
        "git -C /x config user.email",
        "git -C /x rev-parse HEAD",
    ] {
        assert_eq!(decision(cmd, "/here"), "allow", "{cmd}");
    }
}

#[test]
fn write_dash_c_not_auto_allowed() {
    // Not read-only → falls through to prompt unless it targets cwd.
    for cmd in [
        "git -C /other commit -m x",
        "git -C /other branch -d foo",
        "git -C /other branch newbranch",
        "git -C /other tag -d v1",
        "git -C /other tag -a v1 -m msg",
        "git -C /other tag v1",
        "git -C /other config user.email a@b",
        "git -C /other config set user.email a@b",
        "git -C /other config --unset user.email",
        "git -C /other remote add o url",
        "git -C /other remote set-url o url",
        "git -C /other reflog expire --all",
        "git -C /other symbolic-ref HEAD refs/heads/x",
        "git -C /other push",
        "git -C /other checkout main",
    ] {
        assert_eq!(decision(cmd, "/here"), "prompt", "{cmd}");
    }
}

#[test]
fn redundant_dash_c_at_cwd_denied() {
    let cwd = std::env::current_dir().unwrap();
    let cwd = cwd
        .to_str()
        .unwrap();
    let cmd = format!("git -C {cwd} commit -m x");
    assert_eq!(decision(&cmd, cwd), "deny");
    // "." resolves to cwd too.
    assert_eq!(decision("git -C . push", cwd), "deny");
    // Read-only too: the allow only covers it while the whole command qualifies,
    // so anything else in the chain leaves the no-op `-C` at a bare prompt.
    assert_eq!(decision("git -C . log --oneline -3", cwd), "deny");
    assert_eq!(decision("git -C . status; ls src", cwd), "deny");
}
