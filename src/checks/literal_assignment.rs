use crate::checks::shell;
use crate::output::HookOutput;

/// Denies a literal named by one segment and expanded by a later one. One approval
/// covers the whole command string, so the assignment prefix matches no permission
/// rule and the "don't ask again" entry it offers is that one command, value and all.
pub fn check(command: &str) -> Option<HookOutput> {
    let segments = shell::chain_segments(command)?;
    for (i, segment) in segments
        .iter()
        .enumerate()
    {
        for (name, value) in literal_assignments(segment) {
            if segments[i + 1..]
                .iter()
                .any(|later| expands(later, name))
            {
                return Some(HookOutput::deny("PreToolUse", &reason(name, value)));
            }
        }
    }
    None
}

fn reason(name: &str, value: &str) -> String {
    format!(
        "`{name}={value}` then `${name}` blocked. A permission rule matches the command text, and \
that text starts with the assignment — so the call matches nothing that was ever approved, and \
the entry the prompt offers to remember is this exact command with this exact value in it. The \
value is a literal: write it where `${name}` is used and drop the assignment segment. A value \
that has to be computed (`{name}=$(…)`) is not this and keeps its normal prompt."
    )
}

/// The `NAME=value` pairs of a segment that is nothing but assignments. A value
/// holding a substitution is not one: it cannot be written where it is used, and
/// capturing a command's output into a variable is the shape that keeps a
/// credential out of the transcript.
fn literal_assignments(segment: &str) -> Vec<(&str, &str)> {
    if !shell::is_bare_assignment(segment) {
        return Vec::new();
    }
    segment
        .split_whitespace()
        .filter_map(|word| word.split_once('='))
        .filter(|(_, value)| !shell::has_substitution(value))
        .collect()
}

/// Whether the text expands `name`, as `$name` or `${name…}`. Single quotes are not
/// excluded: a `$P` that does not expand leaves the assignment dead, which is not
/// the habit this is aimed at and not worth the false negatives of tracking quoting.
fn expands(text: &str, name: &str) -> bool {
    for (i, _) in text.match_indices('$') {
        let rest = &text[i + 1..];
        let rest = rest
            .strip_prefix('{')
            .unwrap_or(rest);
        let Some(tail) = rest.strip_prefix(name) else {
            continue;
        };
        // A longer name starting with this one is a different variable.
        if !tail.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::check;

    fn denied(command: &str) -> bool {
        check(command).is_some()
    }

    #[test]
    fn a_named_literal_that_is_expanded_is_refused() {
        for cmd in [
            "P=/some/path; pdftotext -layout $P -",
            "P=/some/path && cat ${P}",
            "C=/x; ls $C/src; grep -rn foo $C/src | head",
            "A=1 B=/x; ls $B",
            "DIR=/x ; cd \"$DIR\"",
        ] {
            assert!(denied(cmd), "should deny: {cmd}");
        }
    }

    #[test]
    fn everything_else_keeps_its_prompt() {
        for cmd in [
            // Never expanded: nothing was hidden from the command text.
            "A=1 B=2; grep -rn foo src",
            // A different variable that merely starts with the name.
            "P=/x; echo $PATH",
            // Computed, so it cannot be written where it is used.
            "URI=$(yq -r .uri secrets.yaml); mongosh \"$URI\"",
            // An environment prefix is one command word, not a segment of its own.
            "LANG=C sort file",
            // Expanded before it is named — that is the shell's problem, not a habit.
            "echo $P; P=/x",
            // Not an assignment at all.
            "cd /x; grep -rn foo .",
        ] {
            assert!(!denied(cmd), "should allow: {cmd}");
        }
    }
}
