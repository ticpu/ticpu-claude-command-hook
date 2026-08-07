use crate::checks::shell;
use crate::output::HookOutput;

/// Clients that open a session somewhere this shell cannot see — a remote host or
/// a database server. Everything they do lands outside the working tree, so the
/// call has to be readable on its own.
const SESSION_TOOLS: [&str; 6] = ["ssh", "sshfs", "psql", "mysql", "mariadb", "mongosh"];

const BUNDLED: &str = "must be the whole Bash call — no `;`, `&&`, `||` or `&`. What it does \
lands outside this tree, so it gets read and approved on its own, not behind another command's \
approval; and its session is not this shell's, so a leading `cd` buys it nothing. Split the \
chain into separate calls and pass absolute paths. A pipe into a viewer (`| jq`) is fine, a \
bare `echo` alongside it is fine, and chaining *inside* the quoted remote command or SQL is \
the far end's business.";

const NOT_LEADING: &str = "must be the command that starts the pipeline, not a stage fed by \
another one: whatever produces the input then rides along on this command's approval. Read the \
input from a file instead (`-f file` / `< file`), or run the producer as its own call.";

pub fn check(command: &str) -> Option<HookOutput> {
    let (tool, reason) = decide(command)?;
    Some(HookOutput::deny(
        "PreToolUse",
        &format!("`{tool}` {reason}"),
    ))
}

/// A heredoc body is how these clients are normally fed, and `shell` refuses to
/// split a command carrying one — so the text before the marker is judged instead.
/// The body is data, its `;` and `&&` are the far end's. What this cannot see is a
/// chain operator *after* the terminator; it fails open there rather than guessing.
fn decide(command: &str) -> Option<(&'static str, &'static str)> {
    let head = shell::before_heredoc(command);
    match shell::chain_segments(head) {
        Some(segments) => bundled(head, &segments),
        // Command substitution or an unbalanced quote: no reliable split left, so
        // only the coarse question gets answered — is a client chained with anything.
        None => substituted(head).map(|tool| (tool, BUNDLED)),
    }
}

/// A newline outside quotes separates two commands as surely as `;` does, and
/// `chain_parts` does not split on it. Continuations are already gone: `unquoted`
/// drops the escape and the newline with it.
fn spans_lines(command: &str) -> bool {
    shell::unquoted(command).is_some_and(|bare| {
        bare.trim_end()
            .contains('\n')
    })
}

fn bundled(command: &str, segments: &[&str]) -> Option<(&'static str, &'static str)> {
    // A lone `echo` is not company: it runs nothing and writes nothing, and
    // labelling the call or reporting its `$?` afterwards is routine.
    let company = segments
        .iter()
        .filter(|segment| !shell::is_lone_echo(segment))
        .count();
    let chained = company > 1 || spans_lines(command);
    segments
        .iter()
        .find_map(|segment| {
            let stages = shell::pipeline_stages(segment)?;
            stages
                .iter()
                .enumerate()
                .find_map(|(i, stage)| {
                    let tool = session_tool(shell::command_word(stage)?)?;
                    match (chained, i) {
                        (true, _) => Some((tool, BUNDLED)),
                        (false, 0) => None,
                        (false, _) => Some((tool, NOT_LEADING)),
                    }
                })
        })
}

/// Quoted spans are dropped first, so a client named inside a message or a SQL
/// string is not read as one being run.
fn substituted(head: &str) -> Option<&'static str> {
    let head = shell::unquoted(head).unwrap_or_else(|| head.to_string());
    let tokens: Vec<&str> = head
        .split_whitespace()
        .collect();
    if !tokens
        .iter()
        .any(|token| ends_a_command(token))
    {
        return None;
    }
    tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| shell::starts_a_command(&tokens, *i))
        .find_map(|(_, token)| session_tool(token))
}

/// A token terminating a command with a chain operator. `|` alone is excluded — it
/// pipes rather than chains — but `||` still counts.
fn ends_a_command(token: &str) -> bool {
    token.ends_with(';') || token.ends_with('&') || token.ends_with("||")
}

fn session_tool(word: &str) -> Option<&'static str> {
    let word = word
        .rsplit('/')
        .next()
        .unwrap_or(word);
    SESSION_TOOLS
        .iter()
        .find(|tool| **tool == word)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::check;

    fn denied(command: &str) -> bool {
        check(command).is_some()
    }

    #[test]
    fn denies_a_session_client_bundled_with_anything_else() {
        for cmd in [
            "cd /x && psql -c 'select 1'",
            "psql -c 'select 1'; ls",
            "ssh host uptime && rm -rf /x",
            "ls || ssh host uptime",
            "sshfs host:/x /mnt && ls /mnt",
            "mariadb -e 'show tables'; ls",
            "mongosh --eval 'db.x.find()' & wait",
            "sudo -u postgres psql -c 'select 1' && rm -rf /x",
            "/usr/bin/ssh host uptime; ls",
            "timeout 45 ssh host uptime && rm -rf /x",
            // An echo excuses itself, not a third command.
            "ssh host uptime; echo done; rm -rf /x",
            // Two clients are company for each other.
            "ssh a uptime; ssh b uptime",
            // Not a bare echo: it writes a file / runs what it prints.
            "ssh host uptime; echo done > /x/f",
            "ssh host uptime; echo 'rm -rf /x' | sh",
            // A newline is a separator too, and the splitter does not cut there.
            "ssh host uptime\nls /x",
            // Heredoc: the chain is in the text options are read from.
            "cd /x && psql <<EOF\nselect 1;\nEOF",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn denies_a_client_fed_by_another_command() {
        for cmd in [
            "cat dump.sql | mysql mydb",
            "curl -s https://x/q.sql | psql",
            "echo 'db.x.find()' | mongosh",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn leaves_a_standalone_client_alone() {
        for cmd in [
            "psql -c 'select 1'",
            "psql -f /x/q.sql | jq .",
            "ssh host uptime",
            "ssh host 'cd /x && make'",
            "mysql mydb < /x/dump.sql",
            "mariadb -e 'show tables' | head -20",
            "mongosh --eval 'db.x.find()' | jq .",
            "mysql -e 'show tables' | jq . ; echo done",
            // Reporting the exit status is routine, and `timeout` is a wrapper.
            "timeout 45 ssh -o BatchMode=yes p4 '~/t/prompt-try wofi'; echo \"rc=$? (0=allow)\"",
            "echo '--- schema ---' && psql -c '\\d users'",
            "sudo -u postgres psql -c 'select 1' && echo ok",
            // A heredoc body is data: its statements are not this shell's chain.
            "psql <<EOF\nselect 1; select 2;\nEOF",
            "mongosh <<'EOF'\ndb.x.find() && db.y.find()\nEOF",
            // Continuations are one command.
            "psql \\\n  -c 'select 1'",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn only_the_real_clients_count() {
        for cmd in [
            "cd /x && cargo test",
            "apt list --installed | grep -c mysql",
            // Named, not run.
            "git commit -m 'fix: deny psql && rm'",
            "echo 'ssh host; ls'",
            "cd /x && pg_dump mydb > /x/out.sql",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }
}
