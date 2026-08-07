//! End-to-end verdicts: feeds hook JSON to the built binary and checks what it
//! decides. Unlike the per-check unit tests this exercises dispatch order, the
//! `gf` sibling lookup, and the JSON shape Claude Code actually receives.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

use Verdict::{Allow, Deny, Fold, Pass};

#[derive(Debug, PartialEq)]
enum Verdict<S> {
    /// No output at all: no check objected, so the normal permission prompt applies.
    Pass,
    Deny,
    /// An explicit allow decision: the permission prompt is skipped.
    Allow,
    /// Rewritten command, with `{gf}` standing in for the absolute gf path.
    Fold(S),
}

impl Verdict<&str> {
    fn owned(&self) -> Verdict<String> {
        match self {
            Pass => Pass,
            Deny => Deny,
            Allow => Allow,
            Fold(rewritten) => Fold(rewritten.to_string()),
        }
    }
}

const CASES: &[(&str, Verdict<&str>)] = &[
    (
        "grep -rn \"enum C911pVariable\" -A 60 /x/variables.rs | head -80; ls /x/",
        Fold("grep -rn \"enum C911pVariable\" -A 60 /x/variables.rs | {gf} | head -80 ; ls /x/"),
    ),
    (
        "cd /x && grep -rn foo .",
        Fold("cd /x && { grep -rn foo . | {gf}; (exit ${PIPESTATUS[0]}); }"),
    ),
    (
        "grep -rn a x; grep -rn b y",
        Fold(
            "{ grep -rn a x | {gf}; (exit ${PIPESTATUS[0]}); } ; \
             { grep -rn b y | {gf}; (exit ${PIPESTATUS[0]}); }",
        ),
    ),
    // A segment that cannot be folded costs only itself.
    (
        "grep -rn foo src > out; grep -rn bar src",
        Fold("grep -rn foo src > out ; { grep -rn bar src | {gf}; (exit ${PIPESTATUS[0]}); }"),
    ),
    // The filtering search keeps whole paths to match on; gf runs after it.
    (
        "rg -n --no-heading 'a|b' /x/ | rg -v 'public.xml|internal.xml' | head",
        Fold("rg -n --no-heading 'a|b' /x/ | rg -v 'public.xml|internal.xml' | {gf} | head"),
    ),
    // Merged stderr is not a stdout redirect, so the fold still applies.
    (
        "ls /x/; grep -rn \"a\\|b\" /x/f.xml 2>&1 | head",
        Fold("ls /x/ ; grep -rn \"a\\|b\" /x/f.xml 2>&1 | {gf} | head"),
    ),
    ("grep -rn foo src 2>/dev/null", Deny),
    // rg's -r is --replace: every shape of it rewrites the output instead of recursing.
    ("rg -rn foo src", Deny),
    ("rg -nrl foo src", Deny),
    ("rg -r foo src", Deny),
    ("rg -r -n foo src", Deny),
    (
        "rg -n foo src",
        Fold("rg -n foo src | {gf}; (exit ${PIPESTATUS[0]})"),
    ),
    // A filtering search numbering the piped stream, which is not any file's lines.
    ("rg -n foo src | rg -n bar | head -30", Deny),
    (
        "rg -n foo src | rg bar | head -30",
        Fold("rg -n foo src | rg bar | {gf} | head -30"),
    ),
    // Redirecting stdout keeps the fold off: the file must get the raw output.
    ("grep -rn foo src 2>&1 >out", Pass),
    ("find / -name foo", Deny),
    // `command grep` is the opt-out, even chained or with stderr dropped.
    ("ls -ld /x; command grep -c foo /y", Pass),
    ("command grep -rn foo src 2>/dev/null", Pass),
    ("cargo test 2>&1 | grep -E '^test result'", Pass),
    ("for f in *.rs; do grep -n foo \"$f\"; done", Pass),
    ("grep -rl foo . | xargs sed -i s/a/b/", Pass),
    ("git commit -m 'fix the grep call'", Pass),
    ("git -C /x diff --stat Cargo.lock", Allow),
    // Read-only git runs no hooks, so the `cd` Claude Code warns about is harmless.
    ("cd /x && git diff --stat Cargo.lock", Allow),
    (
        "cd /x && git status --short && git branch --show-current",
        Allow,
    ),
    ("cd /x && git stash pop", Pass),
    ("cd /x && git commit --no-verify -m 'feat: x'", Deny),
    // Staging named paths runs no hook either; the blanket forms are denied.
    (
        "cd /x && git add crates/w/src/mod.rs crates/w/src/queue.rs",
        Allow,
    ),
    ("cd /x && git add -A", Deny),
    ("git add .", Deny),
    // A commit runs the target repo's hooks and the cd buys nothing; the same shape
    // quoted in a message does not count.
    (
        "cd /x/rust && git commit -m \"$(cat <<'EOF'\nrefactor: collapse the rule\nEOF\n)\"",
        Deny,
    ),
    (
        "git commit -F - <<EOF\nfix: deny cd && git commit\nEOF",
        Pass,
    ),
    // Quoting citation ranges: a line-selecting sed and a label add no side effect.
    (
        "cd /x && git show c2c5964:src/a.c | sed -n '2766,2770p' && echo '=== amr' && \
         git show c2c5964:src/b.c | sed -n '688,692p'",
        Allow,
    ),
    ("git show HEAD:a.c | sed -i '1d'", Pass),
    // The next call gets a fresh shell, so a `cd` with no work behind it is noise.
    ("cd /x", Deny),
    ("cd /x && cargo test", Pass),
    // The allow must not cover a second command riding on the same decision.
    ("git -C /x status; rm -rf /y", Pass),
    // A remote/database client shares its approval with whatever it is chained to.
    ("cd /x && psql -c 'select 1'", Deny),
    ("ssh host uptime && rm -rf /x", Deny),
    ("mariadb -e 'show tables'; ls", Deny),
    ("cat dump.sql | mysql mydb", Deny),
    ("psql -f /x/q.sql | jq .", Pass),
    // A bare `echo` is not company, and `timeout` is a wrapper around the client.
    (
        "timeout 45 ssh -o BatchMode=yes p4 'prompt-try wofi'; echo \"rc=$? (0=allow)\"",
        Pass,
    ),
    ("ssh host 'cd /x && make'", Pass),
    ("mongosh <<'EOF'\ndb.x.find() && db.y.find()\nEOF", Pass),
    // A read-only git still reaches the fold, so the allow runs after grep_fold.
    (
        "cd /x && git grep -n foo",
        Fold("cd /x && { git grep -n foo | {gf}; (exit ${PIPESTATUS[0]}); }"),
    ),
];

#[test]
fn verdicts_match() {
    for (command, expected) in CASES {
        assert_eq!(verdict(command), expected.owned(), "command: {command}");
    }
}

fn verdict(command: &str) -> Verdict<String> {
    let stdout = run_hook(command);
    if stdout
        .trim()
        .is_empty()
    {
        return Pass;
    }
    let json: Value = serde_json::from_str(&stdout).expect("hook JSON");
    let specific = &json["hookSpecificOutput"];
    match specific["updatedInput"]["command"].as_str() {
        // Fold back to the placeholder so the expectation stays path-independent.
        Some(rewritten) => Fold(rewritten.replace(&gf_path(), "{gf}")),
        None => match specific["permissionDecision"].as_str() {
            Some("deny") => Deny,
            Some("allow") => Allow,
            _ => panic!("unexpected hook output: {stdout}"),
        },
    }
}

fn run_hook(command: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ticpu-claude-command-hook"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn hook");
    let payload = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "cwd": ".",
        "tool_input": { "command": command },
    });
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(
            payload
                .to_string()
                .as_bytes(),
        )
        .expect("write payload");
    let out = child
        .wait_with_output()
        .expect("hook output");
    assert!(
        out.status
            .success(),
        "hook exited {:?}",
        out.status
    );
    String::from_utf8(out.stdout).expect("utf-8 hook output")
}

/// The sibling `gf` the hook will splice in — `cargo test` builds both binaries,
/// so this exists in debug and release alike.
fn gf_path() -> String {
    PathBuf::from(env!("CARGO_BIN_EXE_ticpu-claude-command-hook"))
        .parent()
        .expect("binary dir")
        .join("gf")
        .to_str()
        .expect("utf-8 path")
        .to_string()
}
