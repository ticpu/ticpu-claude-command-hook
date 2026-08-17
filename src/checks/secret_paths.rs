//! Credential files: refused where the shell would print one, left to the normal
//! prompt where the value is captured instead.

use std::path::PathBuf;

use crate::checks::git_bypass::location::resolve;
use crate::checks::{marker, search_flags, shell};
use crate::input::HookInput;
use crate::output::HookOutput;

/// Words that make a file name say what it holds.
const SECRET_WORDS: &[&str] = &["secret", "credential", "passwd", "password"];

/// Names that carry no such word and hold credentials anyway.
const SECRET_NAMES: &[&str] = &[
    ".pgpass",
    ".netrc",
    "_netrc",
    ".my.cnf",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    "authinfo",
    "kubeconfig",
    "shadow",
];

/// Extensions that identify a key, a certificate bundle or a password store.
const KEY_EXTENSIONS: &[&str] = &[
    ".pem",
    ".key",
    ".p12",
    ".pfx",
    ".jks",
    ".keystore",
    ".kdbx",
    ".ovpn",
];

/// Directories whose contents are credentials whatever the file inside is called.
const SECRET_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".docker",
    ".password-store",
];

/// A name matched on wording alone is exempt here: a module or a page about
/// credentials is not one.
const PROSE_EXTENSIONS: &[&str] = &[
    ".rs", ".py", ".go", ".ts", ".tsx", ".js", ".jsx", ".c", ".h", ".cc", ".cpp", ".hpp", ".java",
    ".rb", ".php", ".pl", ".sh", ".nix", ".md", ".rst", ".adoc", ".txt",
];

/// A dot-separated word marking the file as a stand-in shipped with the values
/// taken out, wherever it sits in the name: `secrets.yaml.sample`, `.env.example`.
const TEMPLATE_WORDS: &[&str] = &["example", "sample", "template", "dist"];

/// Extensions saying the values inside are ciphertext, so reading one spends
/// nothing.
const CIPHERTEXT_EXTENSIONS: &[&str] = &[".eyaml", ".gpg", ".age", ".enc"];

/// Programs that put an argument on screen. Everything else is assumed to use the
/// value it was handed — what it then does with it is beyond this check.
const PRINTERS: &[&str] = &["echo", "printf", "print"];

/// Programs that name a file without opening it: they report or change what the
/// filesystem says about it, and cannot put a byte of its contents anywhere.
const OPENS_NOTHING: &[&str] = &[
    "ls", "stat", "test", "[", "chmod", "chown", "chgrp", "touch", "rm", "mkdir", "realpath",
    "dirname", "basename",
];

/// What turns `git log` into a printer of the file it names.
const PATCH_FLAGS: &[&str] = &[
    "-p",
    "-u",
    "--patch",
    "-c",
    "--cc",
    "--patch-with-stat",
    "--patch-with-raw",
];

/// Flags whose value is a key the program opens for itself.
const KEY_FLAGS: &[&str] = &[
    "-i",
    "--identity-file",
    "--identity",
    "--key",
    "--keyfile",
    "--key-file",
    "--sslkey",
    "--client-key",
    "--private-key",
    "--tlscertificatekeyfile",
    "--ssh-key",
];
/// The waiver for a path this cannot tell from a credential: named in the deny,
/// created by a command the user approves, spent by the next refusal. Its own name
/// carries none of the words above, or the command creating it would be refused.
const WAIVER: &str = "transcript-read-waiver";

pub fn check(input: &HookInput) -> Option<HookOutput> {
    let printed = printed_path(input.command(), &input.cwd)?;
    refuse(&command_reason(&printed))
}

/// `Read` hands back the whole file and `Grep` the matching lines; both land in the
/// transcript, so the path alone decides.
pub fn tool(input: &HookInput) -> Option<HookOutput> {
    let named = match input
        .tool_name
        .as_str()
    {
        "Read" => input.file_path(),
        _ => input.path(),
    };
    let path = secret_path(named, &input.cwd)?;
    refuse(&tool_reason(&path))
}

/// A file the name rules read as a credential and the user knows is not one still
/// has to be readable, so the refusal names its own waiver. Spent only once
/// something has been refused: a marker no refusal consumes is one the next
/// refusal can still spend.
fn refuse(reason: &str) -> Option<HookOutput> {
    if marker::spend(WAIVER) {
        return None;
    }
    Some(HookOutput::deny(
        "PreToolUse",
        &format!(
            "{reason}\nIf this path is not a credential, run `{}` and repeat the call; that \
             waiver is spent on the next refusal.",
            marker::command(WAIVER)
        ),
    ))
}

/// Creating the waiver is prompted whatever the permission rules say, so an
/// allowlisted `touch` cannot hand one out unseen.
pub fn waiver_requested(command: &str) -> Option<HookOutput> {
    marker::creation_requested(command, WAIVER).then(|| {
        HookOutput::ask(
            "PreToolUse",
            "This creates a one-shot pass for the next command or read that names a credential \
             file: it is deleted as it is used. Approve only if that path holds nothing secret \
             on this box — the contents go into the session transcript.",
        )
    })
}

/// The first credential path this command would print, if any. A command that
/// cannot be split is judged whole: this check has no allow to withhold, so being
/// wrong costs a prompt rather than a tool call.
fn printed_path(command: &str, cwd: &str) -> Option<String> {
    shell::chain_segments(command)
        .unwrap_or_else(|| vec![command])
        .iter()
        .find_map(|segment| leaked_by(segment, cwd))
}

fn leaked_by(segment: &str, cwd: &str) -> Option<String> {
    shell::pipeline_stages(segment)
        .unwrap_or_else(|| vec![segment])
        .iter()
        .find_map(|stage| leaked_by_stage(stage, cwd))
}

fn leaked_by_stage(stage: &str, cwd: &str) -> Option<String> {
    let spans = shell::substitution_spans(stage).unwrap_or_default();
    if opens_nothing(stage) {
        return None;
    }
    let patterns = shell::is_searcher(stage)
        .then(|| search_flags::pattern_words(stage))
        .flatten()
        .unwrap_or_default();
    let mut captured = None;
    let all = tokens(stage);
    for (i, (at, token)) in all
        .iter()
        .enumerate()
    {
        let Some(path) = secret_path(token, cwd) else {
            continue;
        };
        if handed_over(&all, i) || patterns.contains(&i) {
            continue;
        }
        if !spans
            .iter()
            .any(|span| span.contains(at))
        {
            return Some(path);
        }
        captured.get_or_insert(path);
    }
    let captured = captured?;
    prints_its_arguments(stage, &spans).then_some(captured)
}

/// The stage names a path without opening it. `git log` reports commits rather
/// than contents, so it belongs here until a flag asks for the patch.
fn opens_nothing(stage: &str) -> bool {
    match shell::program(stage) {
        Some("git") => {
            shell::command_word(stage) == Some("log")
                && shell::program_args(stage).is_some_and(|args| {
                    !args
                        .iter()
                        .any(|arg| prints_a_patch(arg))
                })
        }
        Some(program) => OPENS_NOTHING.contains(&program),
        None => false,
    }
}

fn prints_a_patch(arg: &str) -> bool {
    PATCH_FLAGS.contains(&arg)
        || arg.starts_with("-U")
        || arg.starts_with("-L")
        || arg.starts_with("--unified")
}

/// The path is the value of a flag naming a key for the program to open itself —
/// `ssh -i`, a client's `--sslkey`. The file is read by that program, not by the
/// shell, so nothing of it reaches the output.
fn handed_over(tokens: &[(usize, &str)], i: usize) -> bool {
    let flag = |token: &str| {
        KEY_FLAGS.contains(
            &bare(token)
                .to_ascii_lowercase()
                .as_str(),
        )
    };
    if let Some((_, token)) = tokens.get(i) {
        if let Some((lhs, _)) = bare(token).split_once('=') {
            return flag(lhs);
        }
    }
    i > 0 && flag(tokens[i - 1].1)
}

/// What the segment runs once the substitutions are lifted out of it: an
/// assignment holds the value, a program is handed it, and a printer puts it on
/// screen. A substitution standing on its own as the command runs the file's
/// contents, which prints whatever fails.
fn prints_its_arguments(segment: &str, spans: &[std::ops::Range<usize>]) -> bool {
    let mut outer = String::with_capacity(segment.len());
    let mut at = 0;
    for span in spans {
        outer.push_str(&segment[at..span.start]);
        at = span.end;
    }
    outer.push_str(&segment[at..]);

    match shell::program(&outer) {
        Some(program) => PRINTERS.contains(&program),
        None => !is_assignment(&outer),
    }
}

fn is_assignment(outer: &str) -> bool {
    let mut words = outer
        .split_whitespace()
        .filter(|word| !matches!(*word, "export" | "local" | "declare" | "readonly"))
        .peekable();
    words
        .peek()
        .is_some()
        && words.all(|word| {
            word.contains('=')
                && !word.starts_with('-')
                && !shell::unquote_token(word).starts_with('=')
        })
}

/// Whitespace-separated tokens with the byte offset each starts at, so a token can
/// be placed inside or outside a substitution span.
fn tokens(segment: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (at, c) in segment.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(at),
            (true, Some(from)) => {
                out.push((from, &segment[from..at]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        out.push((from, &segment[from..]));
    }
    out
}

/// The credential path a token names, if it names one. A name that resolves is
/// taken only when it exists, so a search pattern reading like one
/// (`rg 'aws/credentials' .`) does not refuse the search that quotes it. A
/// directory is not weighed that way: it says what is under it whether or not this
/// box has the file.
fn secret_path(token: &str, cwd: &str) -> Option<String> {
    let token = bare(token);
    // `--config=<path>` and `VAR=<path>` both carry the path on the right.
    let path = match token.split_once('=') {
        Some((lhs, rhs)) if is_flag_or_variable(lhs) => shell::unquote_token(rhs),
        _ => token,
    };
    if path.is_empty() {
        return None;
    }
    if in_secret_dir(path) {
        return Some(path.to_string());
    }
    let name = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    if !is_secret_name(&name) {
        return None;
    }
    match resolvable(path, cwd) {
        Some(resolved) => resolved
            .exists()
            .then(|| path.to_string()),
        None => Some(path.to_string()),
    }
}

/// A token with the shell punctuation that can sit against a path stripped off
/// both ends. A leading `$` survives — it names a variable, and a path this cannot
/// resolve is judged on its wording.
fn bare(token: &str) -> &str {
    let token = token.trim_start_matches(['"', '\'', '`']);
    let token = token
        .strip_prefix("$(")
        .unwrap_or(token);
    token
        .trim_start_matches(['"', '\'', '`'])
        .trim_end_matches(['"', '\'', '`', ')', ';', '&', '|'])
}

fn is_flag_or_variable(lhs: &str) -> bool {
    lhs.starts_with('-')
        || (!lhs.is_empty()
            && lhs
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// Where the path will land, for the tokens whose text settles it. A glob or a
/// variable is judged on its wording instead — there is nothing to stat.
fn resolvable(path: &str, cwd: &str) -> Option<PathBuf> {
    if path.contains(['*', '?', '[', '$']) {
        return None;
    }
    match path.strip_prefix("~/") {
        Some(under) => Some(PathBuf::from(std::env::var_os("HOME")?).join(under)),
        None if path == "~" => None,
        None if cwd.is_empty() && !path.starts_with('/') => None,
        None => Some(resolve(path, cwd)),
    }
}

fn in_secret_dir(path: &str) -> bool {
    path.split('/')
        .any(|part| {
            SECRET_DIRS.contains(
                &part
                    .to_ascii_lowercase()
                    .as_str(),
            )
        })
}

fn is_secret_name(name: &str) -> bool {
    if is_template(name) || is_ciphertext(name) {
        return false;
    }
    if KEY_EXTENSIONS
        .iter()
        .any(|ext| name.ends_with(ext))
        || SECRET_NAMES.contains(&name)
        || is_env_file(name)
    {
        return true;
    }
    if is_prose(name) {
        return false;
    }
    name.starts_with("id_")
        || SECRET_WORDS
            .iter()
            .any(|word| name.contains(word))
}

fn is_prose(name: &str) -> bool {
    PROSE_EXTENSIONS
        .iter()
        .any(|ext| name.ends_with(ext))
        || name.ends_with(".pub")
}

fn is_template(name: &str) -> bool {
    name.split('.')
        .any(|part| TEMPLATE_WORDS.contains(&part))
}

fn is_ciphertext(name: &str) -> bool {
    CIPHERTEXT_EXTENSIONS
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// `.envrc` is direnv's script, not an environment file.
fn is_env_file(name: &str) -> bool {
    name.strip_prefix(".env")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

fn command_reason(path: &str) -> String {
    format!(
        "{path} holds credentials and this command prints what it reads. The output is written \
         to the session transcript, which outlives the session, so a value that reaches it is \
         spent and has to be rotated.\n\
         Capture it instead of printing it:\n  \
         VAR=$(yq -r '<field>' {path})\n  \
         <client> \"$VAR\"\n\
         or inline: <client> \"$(yq -r '<field>' {path})\". Either shape prompts normally. Never \
         cat, grep, head or echo the file or the variable — if you need to see it, ask for it to \
         be run outside the session."
    )
}

fn tool_reason(path: &str) -> String {
    format!(
        "{path} holds credentials; the tool result would put its contents in the session \
         transcript, which outlives the session. Read one field in Bash without printing it — \
         VAR=$(yq -r '<field>' {path}) — and use \"$VAR\"."
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{is_secret_name, printed_path, secret_path};

    /// Existence decides the tokens that carry no directory, so the cases that turn
    /// on it need files on disk. Under `target/`, which is not searched or staged.
    fn fixtures() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/secret-paths-fixtures");
        fs::create_dir_all(dir.join("src")).unwrap();
        for name in [
            "fsa-secrets.yaml",
            ".env",
            ".env.example",
            "notes.md",
            "src/read_secret_management.rs",
        ] {
            fs::write(dir.join(name), "x\n").unwrap();
        }
        dir
    }

    fn denies(command: &str) -> bool {
        let cwd = fixtures();
        printed_path(command, &cwd.to_string_lossy()).is_some()
    }

    #[test]
    fn printing_a_credential_file_is_refused() {
        for command in [
            "cat fsa-secrets.yaml",
            "rg -n 'uri' -A2 fsa-secrets.yaml",
            "yq '.eido.uri' fsa-secrets.yaml",
            "cat ~/.ssh/id_ed25519",
            "grep -rn key ~/.ssh/",
            "cat .env",
            "ls -l && head -5 fsa-secrets.yaml",
            "sed -n 's/.*uri: //p' fsa-secrets.yaml | head -1",
            "echo \"$(cat fsa-secrets.yaml)\"",
            "cat fsa-secrets.yaml > /dev/shm/x",
            // Only the pattern is exempt; the paths after it are still read.
            "rg -n 'uri' ~/.ssh/id_rsa",
            "rg -e 'uri' -n ~/.ssh/id_rsa",
            "rg -f patterns.txt ~/.ssh/id_rsa",
            "git log -p -- ~/.ssh/id_rsa",
            "git log -L 1,5:$HOME/.ssh/id_rsa",
            "psql <<EOF\nselect 1;\nEOF\ncat fsa-secrets.yaml",
        ] {
            assert!(denies(command), "should deny: {command}");
        }
    }

    #[test]
    fn capturing_the_value_is_left_to_the_prompt() {
        for command in [
            "URI=$(yq -r '.eido.uri' fsa-secrets.yaml)",
            "export URI=$(yq -r '.eido.uri' fsa-secrets.yaml)",
            "mongosh --quiet \"$(yq -r '.eido.uri' fsa-secrets.yaml)\" --eval 'db.x.count()'",
            "PGPASSWORD=$(yq -r '.pg' fsa-secrets.yaml) psql -h db -c 'select 1'",
        ] {
            assert!(!denies(command), "should pass: {command}");
        }
    }

    #[test]
    fn a_file_about_credentials_is_not_one() {
        for command in [
            "rg -n foo src/read_secret_management.rs",
            // Handed to a program that opens it, or named without being opened.
            "ssh -i ~/.ssh/id_ed25519 host uptime",
            "ssh -o BatchMode=yes -i ~/.ssh/id_rsa host 'run-probe'",
            "curl --key ~/.ssh/id_rsa https://example.test/",
            "chmod 600 ~/.ssh/id_ed25519",
            "ls -l ~/.ssh/",
            "test -f .env",
            "cat src/read_secret_management.rs",
            "rg -n secrets src/",
            "rg -rn 'aws/credentials' notes.md",
            "cat .env.example",
            "cat pkg/fsa/etc/fsa/secrets.yaml.sample",
            "git log --oneline -8 -- data/secrets.eyaml",
            // The pattern is the one argument a search does not open.
            "rg -n 'password|secret|\\*\\*\\*' src/display.rs",
            "rg -n \"\\$SECRET_TOKEN\" .",
            "grep -rn 'id_[a-z]*' notes.md",
            "rg -e 'password|*' -n src",
            "rg --regexp='secrets/*' src",
            "git log --format=%s -- config/id_rsa*",
            "cat notes.md",
            "git commit -m 'fix: read the secret from the env'",
        ] {
            assert!(!denies(command), "should pass: {command}");
        }
    }

    /// The waiver's own name must not read as a credential, or the command that
    /// creates it is refused and the way past a wrong refusal is unreachable.
    #[test]
    fn the_waiver_can_be_asked_for() {
        let touch = super::marker::command(super::WAIVER);
        assert!(printed_path(&touch, "").is_none(), "{touch}");
        assert!(super::waiver_requested(&touch).is_some());
    }

    #[test]
    fn a_bare_word_is_a_pattern_until_it_names_a_file() {
        let cwd = fixtures();
        let cwd = cwd.to_string_lossy();
        assert!(secret_path("secrets.yaml", &cwd).is_none());
        assert!(secret_path("fsa-secrets.yaml", &cwd).is_some());
        // Nothing to stat: judged on its wording.
        assert!(secret_path("$HOME/.ssh/id_rsa", &cwd).is_some());
        assert!(secret_path("~/.ssh/*", &cwd).is_some());
    }

    #[test]
    fn names_that_say_what_they_hold() {
        for name in [
            "fsa-secrets.yaml",
            ".env",
            ".env.local",
            ".pgpass",
            "id_ed25519",
            "server.pem",
            "wildcard.key",
            "vault.kdbx",
            "credentials",
        ] {
            assert!(is_secret_name(name), "should match: {name}");
        }
        for name in [
            "id_ed25519.pub",
            ".env.example",
            ".envrc",
            "secrets.yaml.sample",
            "secrets.sample.yaml",
            "server.sample.pem",
            "secrets.eyaml",
            "credentials.gpg",
            "secrets.txt",
            "secret_paths.rs",
            "secrets.md",
            "keyring.go",
            "main.rs",
        ] {
            assert!(!is_secret_name(name), "should not match: {name}");
        }
    }
}
