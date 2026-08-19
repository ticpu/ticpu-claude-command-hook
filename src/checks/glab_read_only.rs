use crate::checks::shell;
use crate::output::HookOutput;

const READ_ONLY: &str = "read-only glab (auto-allowed by the hook)";

/// Verb pairs that only read. glab's write verbs sit at the same depth as its read
/// ones — `mr merge` beside `mr view` — so the pair is the unit a decision can be
/// made on, and a bare `glab mr` would carry both.
const READ_PAIRS: &[(&str, &str)] = &[
    ("mr", "view"),
    ("mr", "list"),
    ("mr", "diff"),
    ("mr", "checks"),
    ("issue", "view"),
    ("issue", "list"),
    ("ci", "view"),
    ("ci", "list"),
    ("ci", "status"),
    ("ci", "trace"),
    ("ci", "get"),
    ("pipeline", "view"),
    ("pipeline", "list"),
    ("pipeline", "status"),
    ("release", "view"),
    ("release", "list"),
    ("repo", "view"),
    ("repo", "list"),
    ("repo", "contributors"),
    ("snippet", "view"),
    ("snippet", "list"),
    ("label", "list"),
    ("milestone", "list"),
    ("incident", "view"),
    ("incident", "list"),
    ("schedule", "list"),
    ("auth", "status"),
];

/// Single-word invocations that read nothing of the project at all.
const READ_WORDS: &[&str] = &["version", "help"];

/// Flags that turn `glab api` into a write. The method defaults to GET, so a body
/// flag alone is enough to change it, and `--method`/`-X` names it outright.
const API_WRITES: &[&str] = &[
    "-X",
    "--method",
    "-f",
    "--field",
    "-F",
    "--raw-field",
    "--input",
];

/// Auto-allows glab invocations that only read. `glab_skill` still gates the first
/// call of the session — it runs earlier in `dispatch`, so its deny wins and this
/// allow only applies once the marker exists.
///
/// glab is judged here rather than by a `settings.json` prefix rule because `api`
/// is one prefix covering both directions: the method lives in the flags, not the
/// verb, so only argument inspection tells `glab api projects/:id` from the same
/// path with `-X DELETE`.
pub fn allow(command: &str) -> Option<HookOutput> {
    let segments = shell::chain_segments(command)?;
    let mut glab_seen = false;
    for segment in segments {
        if shell::redirects_anything(segment) {
            return None;
        }
        if shell::is_bare_cd(segment) || shell::is_lone_echo(segment) {
            continue;
        }
        if !is_read_only_segment(segment) {
            return None;
        }
        glab_seen = true;
    }
    glab_seen.then(|| HookOutput::allow("PreToolUse", READ_ONLY))
}

/// A pipeline whose producer is a read-only glab and whose later stages only
/// display what they receive — a `| sh` or a path-consuming `xargs` would ride
/// along on this allow.
fn is_read_only_segment(segment: &str) -> bool {
    let Some(stages) = shell::pipeline_stages(segment) else {
        return false;
    };
    let (first, rest) = stages
        .split_first()
        .expect("pipeline_stages never yields an empty list");
    is_read_only_glab(first)
        && rest
            .iter()
            .all(|stage| shell::is_display_only(stage))
}

fn is_read_only_glab(stage: &str) -> bool {
    if shell::program(stage) != Some("glab") {
        return false;
    }
    let Some(args) = shell::program_args(stage) else {
        return false;
    };
    match verb_words(&args).as_slice() {
        [one, ..] if READ_WORDS.contains(one) => true,
        [one, ..] if *one == "api" => api_is_a_read(&args),
        [group, verb, ..] => READ_PAIRS.contains(&(*group, *verb)),
        _ => false,
    }
}

/// The words that name the subcommand, with global flags and their values dropped:
/// `glab --repo x/y mr list` is `mr list`. A separated flag is assumed to take a
/// value, so a boolean one swallows the verb and the pair stops matching — the miss
/// costs a prompt, where reading the value as the verb would not.
fn verb_words<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut words = Vec::new();
    let mut skip_value = false;
    for arg in args {
        if arg.starts_with('-') {
            skip_value = !arg.contains('=');
            continue;
        }
        if skip_value {
            skip_value = false;
            continue;
        }
        words.push(*arg);
    }
    words
}

/// `glab api` reads unless a flag says otherwise. `-X GET` is still a read; any
/// other method, and any body flag, is not.
fn api_is_a_read(args: &[&str]) -> bool {
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let (flag, glued) = match word.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (*word, None),
        };
        if !API_WRITES.contains(&flag) {
            continue;
        }
        if flag != "-X" && flag != "--method" {
            return false;
        }
        let value = glued.or_else(|| {
            words
                .next()
                .copied()
        });
        if !value.is_some_and(|v| v.eq_ignore_ascii_case("GET")) {
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
    fn reading_verbs_and_get_api_calls_are_allowed() {
        for cmd in [
            "glab mr view 2408",
            "glab mr list --author=@me",
            "glab ci status",
            "glab --repo gitlab.cauca.ca/x/y issue list",
            "glab mr diff 2408 | head -40",
            "cd /x && glab release list; echo done",
            "glab api projects/:id/merge_requests --paginate",
            "glab api -X GET projects/:id",
            "glab version",
        ] {
            assert!(allowed(cmd), "{cmd}");
        }
    }

    #[test]
    fn writing_verbs_and_bodied_api_calls_keep_their_prompt() {
        for cmd in [
            // A write verb at the same depth as its reading sibling.
            "glab mr create --fill",
            "glab mr merge 2408",
            "glab mr note 2408 -m x",
            "glab issue close 12",
            // The method and the body are flags, which is why `api` is judged here.
            "glab api -X DELETE projects/:id",
            "glab api --method=POST projects/:id/issues",
            "glab api projects/:id/issues -f title=x",
            // A consumer that acts, and company the allow must not cover.
            "glab mr list | sh",
            "glab mr view 2408 > /x/out",
            "glab mr view 2408 && rm -rf /x",
        ] {
            assert!(!allowed(cmd), "{cmd}");
        }
    }
}
