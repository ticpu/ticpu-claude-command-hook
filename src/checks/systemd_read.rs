//! Read-only `journalctl` and `systemctl`, run here or handed to an `ssh` as the
//! command line the far end runs. Fail-safe throughout: an unrecognized flag,
//! verb or ssh option returns `None` and the call keeps its normal prompt.

use crate::checks::shell;
use crate::output::HookOutput;

const READ: &str = "read-only journalctl/systemctl (auto-allowed by the hook)";

/// Options that only select what is printed. Absent by omission, and so still
/// prompting: everything that trims, rotates or rebuilds the journal, and the
/// two that read a journal this shell cannot see.
const JOURNAL: Options = Options {
    long: &[
        "no-pager",
        "no-hostname",
        "no-full",
        "no-tail",
        "full",
        "all",
        "utc",
        "system",
        "user",
        "quiet",
        "reverse",
        "catalog",
        "pager-end",
        "dmesg",
        "merge",
        "boot",
        "list-boots",
        "case-sensitive",
        "disk-usage",
        "header",
        "verify",
        "list-catalog",
        "dump-catalog",
        "fields",
    ],
    long_value: &[
        "unit",
        "user-unit",
        "lines",
        "since",
        "until",
        "priority",
        "facility",
        "output",
        "output-fields",
        "grep",
        "identifier",
        "namespace",
        "file",
        "directory",
        "field",
    ],
    short: "xerkmqalNb",
    short_value: "utpogSUDFn",
};

/// `--root`, `--host` and `--machine` are absent for the same reason journalctl's
/// are, and every option that only qualifies a write (`--now`, `--force`) with them.
const SYSTEMCTL: Options = Options {
    long: &[
        "no-pager",
        "no-legend",
        "no-ask-password",
        "plain",
        "full",
        "all",
        "value",
        "quiet",
        "user",
        "system",
        "failed",
        "reverse",
        "recursive",
        "show-types",
        "with-dependencies",
    ],
    long_value: &["type", "state", "property", "lines", "output"],
    short: "laqr",
    short_value: "tpno",
};

/// Verbs that report and change nothing. systemctl's writes sit at the same depth
/// as its reads, so the verb is the unit a decision can be made on.
const SYSTEMCTL_READ_VERBS: &[&str] = &[
    "status",
    "show",
    "cat",
    "is-active",
    "is-enabled",
    "is-failed",
    "list-units",
    "list-unit-files",
    "list-timers",
    "list-sockets",
];

/// ssh options that cannot hand the connection a program or a configuration file
/// of its own. A forward (`-L`, `-R`, `-D`, `-W`, `-J`) and `-F` are absent, as is
/// any `-o` naming a setting not below.
const SSH_SHORT: &str = "46qTtnCv";
const SSH_SHORT_VALUE: &str = "plio";

const SSH_OPTIONS: &[&str] = &[
    "BatchMode",
    "ConnectTimeout",
    "ConnectionAttempts",
    "StrictHostKeyChecking",
    "LogLevel",
    "ServerAliveInterval",
    "ServerAliveCountMax",
    "RequestTTY",
    "Port",
    "User",
];

/// Auto-allows a command that only reports on units or reads their journal. A
/// prefix rule in `settings.json` cannot: `journalctl --rotate` and
/// `systemctl restart` share their program with the reads, and `ssh` shares its
/// with every remote command there is — only the arguments separate them.
pub fn allow(command: &str) -> Option<HookOutput> {
    let segments = shell::chain_segments(command)?;
    let mut read_seen = false;
    for segment in segments {
        if shell::redirects_anything(segment) {
            return None;
        }
        if shell::is_bare_cd(segment)
            || shell::is_lone_echo(segment)
            || shell::is_bare_assignment(segment)
        {
            continue;
        }
        if !is_read_segment(segment) {
            return None;
        }
        read_seen = true;
    }
    read_seen.then(|| HookOutput::allow("PreToolUse", READ))
}

fn is_read_segment(segment: &str) -> bool {
    let Some(stages) = shell::pipeline_stages(segment) else {
        return false;
    };
    let (producer, rest) = stages
        .split_first()
        .expect("pipeline_stages never yields an empty list");
    is_read_producer(producer)
        && rest
            .iter()
            .all(|stage| shell::is_harmless_consumer(stage))
}

/// `leading_word`, not `program`: a wrapper must not ride along on this allow,
/// and `sudo journalctl` is a root command whichever verb follows it.
fn is_read_producer(stage: &str) -> bool {
    let args = || shell::program_args(stage).unwrap_or_default();
    match shell::leading_word(stage) {
        Some("journalctl") => journalctl_reads(&args()),
        Some("systemctl") => systemctl_reads(&args()),
        Some("ssh") => remote_body(stage).is_some_and(|body| body_is_a_read(&body)),
        _ => false,
    }
}

fn journalctl_reads(args: &[&str]) -> bool {
    // A bare `-1` is the boot offset `-b` takes, not a flag cluster.
    let args: Vec<&str> = args
        .iter()
        .copied()
        .filter(|arg| !is_offset(arg))
        .collect();
    positionals(&args, &JOURNAL).is_some()
}

fn is_offset(arg: &str) -> bool {
    arg.strip_prefix('-')
        .is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_digit())
        })
}

fn systemctl_reads(args: &[&str]) -> bool {
    positionals(args, &SYSTEMCTL).is_some_and(|words| {
        words
            .first()
            .is_some_and(|verb| SYSTEMCTL_READ_VERBS.contains(verb))
    })
}

/// The command line an `ssh` stage hands the far end, with the enclosing quotes
/// dropped when the whole of it is one quoted span. `None` unless every option
/// before the destination is inert and a command follows it — an ssh with no
/// command opens a shell, which is not a read of anything.
fn remote_body(stage: &str) -> Option<String> {
    let args = shell::program_args(stage)?;
    let mut i = 0;
    while let Some(raw) = args.get(i) {
        let arg = shell::unquote_token(raw);
        let Some(cluster) = arg.strip_prefix('-') else {
            break;
        };
        if cluster.is_empty() || arg.starts_with("--") {
            return None;
        }
        i += consume_option(cluster, &args, i)?;
    }
    let body = args
        .get(i + 1..)?
        .join(" ");
    let body = body.trim();
    (!body.is_empty()).then(|| unquote_whole(body))
}

/// How many arguments one ssh option occupies, or `None` when it is not one this
/// allow covers. A value-taking flag ends its cluster: the rest is its value, or
/// the next argument when nothing is glued on.
fn consume_option(cluster: &str, args: &[&str], i: usize) -> Option<usize> {
    for (at, c) in cluster.char_indices() {
        if SSH_SHORT.contains(c) {
            continue;
        }
        if !SSH_SHORT_VALUE.contains(c) {
            return None;
        }
        let glued = &cluster[at + c.len_utf8()..];
        let (value, width) = match glued.is_empty() {
            true => (shell::unquote_token(args.get(i + 1)?), 2),
            false => (glued, 1),
        };
        return (c != 'o' || option_is_inert(value)).then_some(width);
    }
    Some(1)
}

fn option_is_inert(value: &str) -> bool {
    value
        .split_once('=')
        .is_some_and(|(name, _)| {
            SSH_OPTIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(name))
        })
}

fn unquote_whole(body: &str) -> String {
    match shell::quoted_spans(body).as_deref() {
        Some([span]) if span.start == 0 && span.end == body.len() => shell::unquote_token(body),
        _ => body,
    }
    .to_string()
}

/// The far end's own chaining is left to the far end when deciding what to
/// refuse; it cannot be left there when deciding what to grant, so the body has
/// to be one pipeline reading one thing. A substitution counts here even though
/// the outer gate saw none — quoted locally, it is the remote shell that runs it.
fn body_is_a_read(body: &str) -> bool {
    if shell::has_substitution(body) {
        return false;
    }
    let Some(segments) = shell::chain_segments(body) else {
        return false;
    };
    matches!(
        segments.as_slice(),
        [only] if !shell::redirects_anything(only) && is_read_segment(only)
    )
}

struct Options {
    long: &'static [&'static str],
    long_value: &'static [&'static str],
    short: &'static str,
    short_value: &'static str,
}

/// The positional words, or `None` as soon as a flag is one the lists do not
/// carry. Cluster-aware in the way `search_flags` is: a short flag taking a value
/// ends its cluster and swallows the next word when nothing is glued on. A flag
/// wrongly assumed to take one swallows a word that then goes unread, which costs
/// a prompt — a systemctl verb read as a value leaves no verb at all.
fn positionals<'a>(args: &[&'a str], opts: &Options) -> Option<Vec<&'a str>> {
    let mut words = Vec::new();
    let mut expect_value = false;
    for raw in args {
        if expect_value {
            expect_value = false;
            continue;
        }
        let arg = shell::unquote_token(raw);
        if let Some(name) = arg.strip_prefix("--") {
            let (name, glued) = match name.split_once('=') {
                Some((name, _)) => (name, true),
                None => (name, false),
            };
            if opts
                .long_value
                .contains(&name)
            {
                expect_value = !glued;
                continue;
            }
            if !opts
                .long
                .contains(&name)
            {
                return None;
            }
            continue;
        }
        if let Some(cluster) = arg.strip_prefix('-') {
            if !cluster_reads(cluster, opts, &mut expect_value) {
                return None;
            }
            continue;
        }
        words.push(arg);
    }
    Some(words)
}

fn cluster_reads(cluster: &str, opts: &Options, expect_value: &mut bool) -> bool {
    if cluster.is_empty() {
        return false;
    }
    for (at, c) in cluster.char_indices() {
        if opts
            .short_value
            .contains(c)
        {
            *expect_value = at + c.len_utf8() == cluster.len();
            return true;
        }
        if !opts
            .short
            .contains(c)
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::allow;

    fn allowed(command: &str) -> bool {
        allow(command).is_some()
    }

    #[test]
    fn reading_a_journal_or_a_unit_is_allowed() {
        for cmd in [
            "journalctl -u sshd -n 100",
            "journalctl -u sshd.service --since '2026-08-01 10:00' --no-pager",
            "journalctl -xe -u foo",
            "journalctl -b -1 -u foo",
            "journalctl _SYSTEMD_UNIT=sshd.service -o cat",
            "systemctl status sshd",
            "systemctl --user status foo.service",
            "systemctl list-units --type=service --state=failed",
            "systemctl cat foo | head -40",
            "journalctl -u foo | grep -i error",
            "journalctl -u foo | wc -l",
            "cd /x && systemctl status foo; echo done",
            "UNIT=foo.service; systemctl status foo.service",
        ] {
            assert!(allowed(cmd), "{cmd}");
        }
    }

    #[test]
    fn the_same_reads_over_ssh_are_allowed() {
        for cmd in [
            "ssh srv journalctl -u sshd -n 200",
            "ssh -o BatchMode=yes srv systemctl status sshd",
            "ssh -p 2222 -l ops srv journalctl -u foo",
            "ssh -q -o ConnectTimeout=5 srv 'journalctl -u foo | grep -i fail'",
            "ssh srv 'systemctl status foo' | tail -20",
            "ssh ops@srv journalctl -u foo --no-pager",
        ] {
            assert!(allowed(cmd), "{cmd}");
        }
    }

    #[test]
    fn a_write_verb_or_an_unlisted_flag_keeps_its_prompt() {
        for cmd in [
            "journalctl --rotate",
            "journalctl --vacuum-size=1G",
            "journalctl --flush",
            "systemctl restart foo",
            "systemctl enable --now foo",
            "systemctl status foo --root=/mnt",
            // Follow never returns, which is the worst way to be wrong here.
            "journalctl -u foo -f",
            "journalctl -fu foo",
            // A wrapper is not covered, and root is what sudo makes of the verb.
            "sudo journalctl -u foo",
            "sudo systemctl status foo",
            "timeout 30 journalctl -u foo",
            // The allow decides the whole call.
            "journalctl -u foo > out",
            "journalctl -u foo | tee out",
            "systemctl status foo | sh",
            "systemctl status foo && rm -rf /x",
        ] {
            assert!(!allowed(cmd), "{cmd}");
        }
    }

    #[test]
    fn an_ssh_that_carries_more_than_the_read_keeps_its_prompt() {
        for cmd in [
            // An option that hands the connection a program or a file.
            "ssh -o 'ProxyCommand=nc %h %p' srv journalctl -u foo",
            "ssh -F ./cfg srv journalctl -u foo",
            "ssh -L 8080:localhost:80 srv journalctl -u foo",
            "ssh -J jump srv journalctl -u foo",
            // The far end's chain cannot be vouched for.
            "ssh srv 'journalctl -u foo && rm -rf /x'",
            "ssh srv 'journalctl -u foo; systemctl restart foo'",
            "ssh srv 'journalctl -u foo > /etc/motd'",
            // Quoted here, run there.
            "ssh srv 'journalctl -u $(cat /etc/hostname)'",
            // No command at all is a shell.
            "ssh srv",
            "ssh -o BatchMode=yes srv",
            // Neither end takes a wrapper.
            "ssh srv sudo journalctl -u foo",
            "sudo ssh srv journalctl -u foo",
            "ssh srv systemctl restart foo",
            "ssh srv journalctl -f -u foo",
        ] {
            assert!(!allowed(cmd), "{cmd}");
        }
    }
}
