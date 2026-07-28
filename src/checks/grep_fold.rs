use std::path::PathBuf;

use serde_json::Value;

use crate::input::HookInput;
use crate::output::HookOutput;

/// Sent on every rewritten grep, so it stays one line; `gf --help` has the rest.
const REASON: &str = "piped through gf: repeated paths folded, `base:` announces what was stripped";

/// Shell metacharacters that make the command something other than a plain
/// pipeline. Rewriting around them would change what runs, so we leave it alone.
const UNSAFE_CHARS: [char; 6] = ['&', ';', '`', '<', '>', '\n'];

/// Later pipeline stages that only display what they receive. Anything else may
/// parse the path off each line — folding would feed it truncated paths.
const DISPLAY_ONLY: [&str; 5] = ["head", "tail", "less", "cat", "nl"];

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

fn rewrite(command: &str, gf: &str) -> Option<String> {
    if command.contains(UNSAFE_CHARS) || command.contains("$(") || command.contains("||") {
        return None;
    }
    let stages: Vec<&str> = command
        .split('|')
        .map(str::trim)
        .collect();
    if stages
        .iter()
        .any(|s| s.is_empty())
    {
        return None;
    }
    if !searches_files(first_word(stages[0], true)?) {
        return None;
    }
    if stages[0]
        .split_whitespace()
        .any(changes_output_shape)
    {
        return None;
    }
    if !stages[1..]
        .iter()
        .all(|s| first_word(s, false).is_some_and(|w| DISPLAY_ONLY.contains(&w)))
    {
        return None;
    }

    Some(match stages.len() {
        // Sole command: the pipeline's status would become gf's, hiding grep's
        // "no match" exit 1 and its errors.
        1 => format!("{} | {gf}; (exit ${{PIPESTATUS[0]}})", stages[0]),
        _ => format!("{} | {gf} | {}", stages[0], stages[1..].join(" | ")),
    })
}

/// First word of a stage, skipping `VAR=value` prefixes, and stepping over `git`
/// so `git grep` is classified on its subcommand.
fn first_word(stage: &str, skip_git: bool) -> Option<&str> {
    let mut words = stage
        .split_whitespace()
        .skip_while(|w| w.contains('=') && !w.starts_with('-'));
    let word = basename(words.next()?);
    if skip_git && word == "git" {
        return words
            .next()
            .map(basename);
    }
    Some(word)
}

fn basename(word: &str) -> &str {
    word.rsplit('/')
        .next()
        .unwrap_or(word)
}

fn searches_files(word: &str) -> bool {
    matches!(
        word,
        "grep" | "egrep" | "fgrep" | "rgrep" | "ugrep" | "ug" | "rg"
    )
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

    #[test]
    fn leaves_non_pipelines_alone() {
        for cmd in [
            "grep -rn foo . && echo hit",
            "grep -rn foo . > out",
            "grep -rn foo . 2>/dev/null",
            "grep -rn foo . || true",
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

    #[test]
    fn does_not_stack_on_an_existing_gf() {
        assert_eq!(rewrite("grep -rn foo src | /opt/hook/gf", GF), None);
    }
}
