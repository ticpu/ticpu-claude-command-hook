//! End-to-end verdicts: feeds hook JSON to the built binary and checks what it
//! decides. Unlike the per-check unit tests this exercises dispatch order, the
//! `gf` sibling lookup, and the JSON shape Claude Code actually receives.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

use Verdict::{Allow, Ask, Deny, Fold, Pass};

#[derive(Debug, PartialEq)]
enum Verdict<S> {
    /// No output at all: no check objected, so the normal permission prompt applies.
    Pass,
    Deny,
    /// An explicit allow decision: the permission prompt is skipped.
    Allow,
    /// The user is prompted whatever the permission rules say.
    Ask,
    /// Rewritten command, with `{gf}` standing in for the absolute gf path.
    Fold(S),
}

impl Verdict<&str> {
    fn owned(&self) -> Verdict<String> {
        match self {
            Pass => Pass,
            Deny => Deny,
            Allow => Allow,
            Ask => Ask,
            Fold(rewritten) => Fold(rewritten.to_string()),
        }
    }
}

const CASES: &[(&str, Verdict<&str>)] = &[
    // The waiver for a judged objection: always prompted, so an allowlisted
    // `touch` cannot hand one out unseen.
    (
        "touch \"$XDG_RUNTIME_DIR/claude-hooks/design-rationale-judge-bypass\"",
        Ask,
    ),
    ("touch \"$XDG_RUNTIME_DIR/claude-hooks/glab-skill-x\"", Pass),
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
    // A rewrite carries an allow for the whole call, so a segment the fold cannot
    // vouch for forfeits the fold instead of being granted permission by it.
    ("grep -rn foo src > out; grep -rn bar src", Pass),
    ("grep -rn foo src; rm -rf /zztest", Pass),
    ("grep -rn foo src && git push origin master", Pass),
    ("grep -rn foo src\nrm -rf /zztest", Pass),
    // A read-only utility alongside is vouched for, so that chain still folds.
    (
        "ls -l /x; grep -rn bar src",
        Fold("ls -l /x ; { grep -rn bar src | {gf}; (exit ${PIPESTATUS[0]}); }"),
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
    // The paths must exist, so these name real files in this repo.
    // The `cd` moves where the add runs, so the paths are spelled from there.
    (
        concat!(
            "cd ",
            env!("CARGO_MANIFEST_DIR"),
            " && git add src/checks/shell.rs src/main.rs"
        ),
        Allow,
    ),
    ("cd /x && git add -A", Deny),
    ("git add .", Deny),
    // Quoting a blanket pathspec changes nothing for git.
    ("git add \".\"", Deny),
    ("git add '*'", Deny),
    ("sudo git add -A", Deny),
    // A directory or a variable sweeps whatever is under it.
    ("git add src", Pass),
    ("git add \"$PWD\"", Pass),
    // git is git however it is reached.
    ("/usr/bin/git commit --no-verify -m \"feat: x\"", Deny),
    ("{ git commit --no-verify -m \"feat: x\"; }", Deny),
    ("git commit -n -m \"feat: x\"", Deny),
    ("git commit \"--no-verify\" -m \"feat: x\"", Deny),
    ("git -c 'commit.gpgsign=false' commit -m \"feat: x\"", Deny),
    ("git -c commit.gpgsign=off commit -m \"feat: x\"", Deny),
    ("cargo build\ngit commit --no-verify -m \"feat: x\"", Deny),
    // An option that writes a file or runs a program is not read-only.
    ("git diff --output=/zztest/pwned", Pass),
    ("git grep --open-files-in-pager=rm -n foo", Pass),
    ("git -c core.pager=rm log", Pass),
    // `2>` truncates the file it names, so the allow must not cover it.
    ("git log 2>/zztest/clobbered", Pass),
    // A search whose path comes from a substitution is still a search.
    ("grep -rn foo $(pwd) 2>/dev/null", Deny),
    ("rg -rn foo $(pwd)", Deny),
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
    // The working directory persists between calls, so moving it is the work.
    ("cd /x", Allow),
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
        "timeout 45 ssh -o BatchMode=yes host 'run-probe'; echo \"rc=$? (0=allow)\"",
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

/// Judged from a subdirectory of this repo, which `CASES` cannot express: a
/// pathspec spelled from the repo root only exists as a shape below the root.
const SUBDIR_CASES: &[(&str, Verdict<&str>)] = &[
    ("git add src/main.rs", Deny),
    ("git add main.rs", Allow),
    // Resolves from neither the subdirectory nor the root: a deletion, or a typo.
    ("git add src/gone.rs", Pass),
];

#[test]
fn verdicts_match() {
    for (command, expected) in CASES {
        assert_eq!(
            verdict(command, "."),
            expected.owned(),
            "command: {command}"
        );
    }
    let subdir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    for (command, expected) in SUBDIR_CASES {
        assert_eq!(
            verdict(command, subdir),
            expected.owned(),
            "command: {command}"
        );
    }
}

/// Edits to a `design-rationale.md`, judged by the countable rules alone. Every row
/// is decided before the model is consulted, so the table needs no ollama — the
/// judge itself is covered by the ignored test beside it.
const EDIT_CASES: &[(&str, Verdict<&str>)] = &[
    (
        "## Why we split the parser\n\nA body long enough to clear the floor, with several \
      more words after it so nothing is skipped for being short.\n",
        Deny,
    ),
    (
        "## A rule worth stating\n\nCLAUDE.md already covers this, which is exactly why the \
      section must not say so, and this body clears the floor.\n",
        Deny,
    ),
    // Short enough that there is no prose to judge — still the user's to approve,
    // since approving is the review and nothing asks for one after the write.
    ("", Ask),
    ("## A heading rename with no body\n", Ask),
];

#[test]
fn edit_verdicts_match() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/design-rationale.md");
    for (added, expected) in EDIT_CASES {
        assert_eq!(
            edit_verdict(path, added),
            expected.owned(),
            "added: {added}"
        );
    }
    // A file this check has no business in never reaches either rule.
    assert_eq!(
        edit_verdict("/x/README.md", "## Why not\n\nlong body here"),
        Pass
    );
}

/// The judge itself, which needs ollama up with the model resident:
/// `cargo test -- --ignored`. Only that it reaches a prompt carrying the objection
/// is asserted — the judge is a model, so which rules it cites varies, and pinning
/// that would buy a flaky test instead of a signal.
#[test]
#[ignore = "needs a local ollama with the judge model resident"]
fn the_judge_objects_to_prose_the_rules_forbid() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/design-rationale.md");
    let added = "## Buffer sizing in the frame reader\n\nTCP guarantees ordered delivery but \
not message framing, so a reader has to cope with partial reads and re-assemble frames \
itself. Previously the reader used a fixed 4096-byte buffer, and an earlier version grew it \
on demand. The consequence is that frames larger than the buffer were split across reads.\n";
    assert_eq!(edit_verdict(path, added), Deny);
    let reason = edit_reason(path, added);
    assert!(reason.contains("judge objects"), "{reason}");
    assert!(reason.contains("Rule "), "{reason}");
    // The deny has to carry the way past it, or the objection is unappealable.
    assert!(reason.contains("design-rationale-judge-bypass"), "{reason}");
}

fn edit_reason(file_path: &str, new_string: &str) -> String {
    let payload = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Edit",
        "cwd": env!("CARGO_MANIFEST_DIR"),
        "tool_input": { "file_path": file_path, "old_string": "", "new_string": new_string },
    });
    let json: Value = serde_json::from_str(&feed(&payload)).expect("hook JSON");
    json["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .expect("a decision carries a reason")
        .to_string()
}

fn edit_verdict(file_path: &str, new_string: &str) -> Verdict<String> {
    let payload = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Edit",
        "cwd": env!("CARGO_MANIFEST_DIR"),
        "tool_input": { "file_path": file_path, "old_string": "", "new_string": new_string },
    });
    let stdout = feed(&payload);
    if stdout
        .trim()
        .is_empty()
    {
        return Pass;
    }
    let json: Value = serde_json::from_str(&stdout).expect("hook JSON");
    match json["hookSpecificOutput"]["permissionDecision"].as_str() {
        Some("deny") => Deny,
        Some("allow") => Allow,
        Some("ask") => Ask,
        _ => panic!("unexpected hook output: {stdout}"),
    }
}

fn verdict(command: &str, cwd: &str) -> Verdict<String> {
    let stdout = run_hook(command, cwd);
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
            Some("ask") => Ask,
            _ => panic!("unexpected hook output: {stdout}"),
        },
    }
}

fn run_hook(command: &str, cwd: &str) -> String {
    feed(&serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "cwd": cwd,
        "tool_input": { "command": command },
    }))
}

fn feed(payload: &Value) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ticpu-claude-command-hook"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn hook");
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
