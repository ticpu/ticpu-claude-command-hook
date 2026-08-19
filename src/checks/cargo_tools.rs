use crate::checks::shell;
use crate::output::HookOutput;

const BUILDS: &str = "cargo build/report verb with display-only consumers \
                      (auto-allowed by the hook)";

/// Verbs that compile the workspace and report. Deliberately narrower than the
/// `settings.json` allowlist: `run` executes whatever the crate names, `add`/`remove`/
/// `update` rewrite the manifest, `clean` deletes, and `publish` leaves the machine —
/// none of those need this allow, because none of them are what a status label is
/// chained to. Widening this list is a permission decision, not a parsing one.
const REPORTING: &[&str] = &[
    "build",
    "check",
    "clippy",
    "test",
    "bench",
    "doc",
    "rustdoc",
    "fmt",
    "tree",
    "metadata",
    "info",
    "search",
    "expand",
    "llvm-cov",
    "semver-checks",
];

/// Auto-allows a cargo build piped into display-only stages, with the usual
/// do-nothing segments around it. `settings.json` grants these verbs too, but its
/// prefix rules cannot decide a command Claude Code refuses to evaluate statically:
/// `${PIPESTATUS[0]}` in the status label is a variable with an array subscript, and
/// a subscript is arith-evaluated, so a prefix match would have to assume what it
/// expands to. A hook allow answers the whole command instead of matching its text,
/// which is why the label no longer costs a confirmation.
pub fn allow(command: &str) -> Option<HookOutput> {
    let segments = shell::chain_segments(command)?;
    let mut cargo_seen = false;
    for segment in segments {
        // `2>&1` merges stderr into the pipe and is the point of the shape; a
        // redirect naming a path would truncate it.
        if shell::redirects_stdout(segment) || names_a_redirect_target(segment) {
            return None;
        }
        if shell::is_bare_cd(segment) || shell::is_lone_echo(segment) {
            continue;
        }
        if !is_reporting_segment(segment) {
            return None;
        }
        cargo_seen = true;
    }
    cargo_seen.then(|| HookOutput::allow("PreToolUse", BUILDS))
}

/// Any redirect whose target is not the `&1`/`&2` fd duplication `2>&1` performs.
fn names_a_redirect_target(segment: &str) -> bool {
    let Some(bare) = shell::unquoted(segment) else {
        return true;
    };
    bare.match_indices('>')
        .any(|(i, _)| {
            !bare[i + 1..]
                .trim_start()
                .starts_with('&')
        })
}

fn is_reporting_segment(segment: &str) -> bool {
    let Some(stages) = shell::pipeline_stages(segment) else {
        return false;
    };
    let (first, rest) = stages
        .split_first()
        .expect("pipeline_stages never yields an empty list");
    is_reporting_cargo(first)
        && rest
            .iter()
            .all(|stage| is_reporting_consumer(stage))
}

/// A search qualifies here and not in `grep_fold`: nothing folds a cargo pipeline,
/// so there is no stripped prefix a later pattern could match against.
fn is_reporting_consumer(stage: &str) -> bool {
    shell::is_display_only(stage) || shell::is_searcher(stage) || shell::is_read_only_util(stage)
}

fn is_reporting_cargo(stage: &str) -> bool {
    if shell::program(stage) != Some("cargo") {
        return false;
    }
    shell::program_args(stage).is_some_and(|args| {
        args.iter()
            // `+nightly` selects a toolchain; the verb is the word after it.
            .find(|word| !word.starts_with(['-', '+']))
            .is_some_and(|verb| REPORTING.contains(verb))
    })
}

#[cfg(test)]
mod tests {
    use super::allow;

    fn allowed(command: &str) -> bool {
        allow(command).is_some()
    }

    #[test]
    fn a_build_with_a_status_label_needs_no_prompt() {
        for cmd in [
            "cargo test",
            "cd /x && cargo test --no-run --offline 2>&1 | grep -E 'error' | head -5; \
             echo \"=== exit ${PIPESTATUS[0]} ===\"",
            "cargo clippy --message-format=short 2>&1 | tail -10",
            "cargo +nightly-2024 build",
        ] {
            assert!(allowed(cmd), "{cmd}");
        }
    }

    #[test]
    fn a_verb_or_consumer_that_acts_keeps_its_prompt() {
        for cmd in [
            // Not a reporting verb: these run, rewrite or publish.
            "cargo run --example x",
            "cargo add serde",
            "cargo publish --dry-run",
            "cargo clean",
            // The consumer, not the producer, is the problem.
            "cargo test | sh",
            "cargo metadata | xargs rm",
            // A redirect that names a path truncates it; `2>&1` does not.
            "cargo build 2>/x/log",
            "cargo build > /x/log",
            // Company this allow does not extend to.
            "cargo test && rm -rf /x",
        ] {
            assert!(!allowed(cmd), "{cmd}");
        }
    }
}
