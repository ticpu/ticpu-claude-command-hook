//! Just enough shell parsing to tell a plain pipeline from everything else.
//! Quote-aware, and it refuses to guess: anything with command substitution or a
//! heredoc comes back as `None` so callers fail open instead of mis-splitting.

/// Commands that read files and print `path:line:` — the ones worth folding.
const SEARCHERS: [&str; 7] = ["grep", "egrep", "fgrep", "rgrep", "ugrep", "ug", "rg"];

/// Wrappers that take the real command as their arguments. `command` is
/// deliberately absent: writing `command grep` is how a caller asks for the real
/// binary and to be left alone, so it must not classify as a search.
const WRAPPERS: [&str; 5] = ["env", "sudo", "time", "nice", "stdbuf"];

/// Later pipeline stages that only display what they receive. Anything else may
/// parse the path off each line — folding would feed it truncated paths — or,
/// for the read-only classifier, run arbitrary code on the piped output.
/// `wc` would count folded lines, and `sort -o <file>` writes a file.
const DISPLAY_ONLY: [&str; 5] = ["head", "tail", "less", "cat", "nl"];

/// Top-level segments paired with the operator that follows each one (`""` for
/// the last), so a caller can rewrite one segment and rebuild the command.
pub fn chain_parts(command: &str) -> Option<Vec<(&str, &str)>> {
    split(command, |b, i| match b[i] {
        b';' => Some(1),
        b'&' if b.get(i + 1) == Some(&b'&') => Some(2),
        b'&' if amp_joins_a_redirect(b, i) => None,
        b'&' => Some(1),
        b'|' if b.get(i + 1) == Some(&b'|') => Some(2),
        _ => None,
    })
}

/// `2>&1`, `>&2` and `&>file` all carry an `&` that is part of a redirection, not
/// a background/chain operator — splitting there would cut a segment in half.
fn amp_joins_a_redirect(b: &[u8], i: usize) -> bool {
    if b.get(i + 1) == Some(&b'>') {
        return true;
    }
    b[..i]
        .iter()
        .rposition(|c| !c.is_ascii_whitespace())
        .is_some_and(|prev| matches!(b[prev], b'>' | b'<' | b'|'))
}

/// Top-level `;`, `&&`, `||`, `&` segments. One segment means no chaining.
pub fn chain_segments(command: &str) -> Option<Vec<&str>> {
    Some(
        chain_parts(command)?
            .into_iter()
            .map(|(segment, _)| segment)
            .collect(),
    )
}

/// Stages of one pipeline segment, split on top-level `|`.
pub fn pipeline_stages(segment: &str) -> Option<Vec<&str>> {
    Some(
        split(segment, |b, i| {
            (b[i] == b'|' && b.get(i + 1) != Some(&b'|')).then_some(1)
        })?
        .into_iter()
        .map(|(stage, _)| stage)
        .collect(),
    )
}

/// True when a top-level redirect can carry stdout (`>`, `>>`, `&>`, `>&2`, any fd
/// but 2) — and when the command cannot be analyzed, since a missed stdout redirect
/// would send rewritten output somewhere it gets read back. `2>`, `2>&1` and `<`
/// leave stdout alone, so they come back false.
pub fn redirects_stdout(command: &str) -> bool {
    split(command, |b, i| {
        (b[i] == b'>' && !preceded_by(b, i, b'>') && !stderr_fd(b, i)).then_some(1)
    })
    .is_none_or(|parts| parts.len() > 1)
}

/// `>>` is one operator; its second byte must not count as another redirect.
fn preceded_by(b: &[u8], i: usize, c: u8) -> bool {
    i > 0 && b[i - 1] == c
}

/// A bare `2` glued to the `>`. A longer number is a higher fd, deliberately left
/// on the stdout side: fail-safe beats guessing what an exotic fd does.
fn stderr_fd(b: &[u8], i: usize) -> bool {
    preceded_by(b, i, b'2')
        && !(i > 1
            && b[i - 2].is_ascii_digit())
}

/// The command word of a stage: `VAR=v` assignments and wrappers are stepped
/// over, and `git` yields its subcommand so `git grep` classifies as a search.
pub fn command_word(stage: &str) -> Option<&str> {
    let mut words = stage.split_whitespace();
    loop {
        let word = basename(words.next()?);
        if word == "git" {
            return words
                .next()
                .map(basename);
        }
        if WRAPPERS.contains(&word) || (word.contains('=') && !word.starts_with('-')) {
            continue;
        }
        return Some(word);
    }
}

/// True when this pipeline's first stage is a file search.
pub fn is_search(segment: &str) -> bool {
    pipeline_stages(segment).is_some_and(|stages| is_searcher(stages[0]))
}

/// True when one pipeline stage is a search — as a later stage it filters lines
/// rather than reading files, but it is still matching against whole paths.
pub fn is_searcher(stage: &str) -> bool {
    command_word(stage).is_some_and(|w| SEARCHERS.contains(&w))
}

/// True when a pipeline stage only displays what it is handed.
pub fn is_display_only(stage: &str) -> bool {
    command_word(stage).is_some_and(|w| DISPLAY_ONLY.contains(&w))
}

fn basename(word: &str) -> &str {
    word.rsplit('/')
        .next()
        .unwrap_or(word)
}

fn analyzable(command: &str) -> bool {
    !command.contains("$(") && !command.contains('`') && !command.contains("<<")
}

/// The command with quoted spans dropped, so a *pattern* containing shell syntax
/// is never read as shell syntax. `None` on an unbalanced quote.
pub fn unquoted(command: &str) -> Option<String> {
    let mask = unquoted_mask(command)?;
    let bytes: Vec<u8> = command
        .bytes()
        .zip(mask)
        .filter_map(|(b, keep)| keep.then_some(b))
        .collect();
    String::from_utf8(bytes).ok()
}

/// Marks the bytes that are outside quotes and are not escapes — the only ones
/// that can be shell operators. Sole quote parser in this module.
fn unquoted_mask(s: &str) -> Option<Vec<bool>> {
    let b = s.as_bytes();
    let mut mask = vec![false; b.len()];
    let mut i = 0;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        match quote {
            // Single quotes take everything literally, including backslashes.
            Some(q) => {
                if b[i] == b'\\' && q == b'"' {
                    i += 2;
                    continue;
                }
                if b[i] == q {
                    quote = None;
                }
                i += 1;
            }
            None => match b[i] {
                b'\\' => i += 2,
                b'\'' | b'"' => {
                    quote = Some(b[i]);
                    i += 1;
                }
                _ => {
                    mask[i] = true;
                    i += 1;
                }
            },
        }
    }
    quote
        .is_none()
        .then_some(mask)
}

/// Splits on the operators `op` reports, pairing each part with the operator text
/// that terminated it so the command can be rebuilt verbatim.
fn split(s: &str, op: impl Fn(&[u8], usize) -> Option<usize>) -> Option<Vec<(&str, &str)>> {
    if !analyzable(s) {
        return None;
    }
    let mask = unquoted_mask(s)?;
    let b = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match mask[i]
            .then(|| op(b, i))
            .flatten()
        {
            Some(len) => {
                parts.push((s[start..i].trim(), &s[i..i + len]));
                i += len;
                start = i;
            }
            None => i += 1,
        }
    }
    parts.push((s[start..].trim(), ""));
    parts
        .iter()
        .all(|(part, _)| !part.is_empty())
        .then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_chains_but_not_pipes() {
        assert_eq!(
            chain_segments("grep x . | head").unwrap(),
            ["grep x . | head"]
        );
        assert_eq!(
            chain_segments("cd /x && grep y .").unwrap(),
            ["cd /x", "grep y ."]
        );
        assert_eq!(chain_segments("a; b || c").unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn a_redirect_amp_is_not_a_chain() {
        assert_eq!(
            chain_segments("rg -n x /y >/dev/null 2>&1").unwrap(),
            ["rg -n x /y >/dev/null 2>&1"]
        );
        assert_eq!(
            chain_segments("cargo build &>log").unwrap(),
            ["cargo build &>log"]
        );
        assert_eq!(
            chain_segments("sleep 1 & wait").unwrap(),
            ["sleep 1", "wait"]
        );
    }

    #[test]
    fn quoted_operators_are_literal() {
        assert_eq!(
            chain_segments("grep -rn 'a;b' .").unwrap(),
            ["grep -rn 'a;b' ."]
        );
        assert_eq!(
            chain_segments("grep -rn \"a && b\" .").unwrap(),
            ["grep -rn \"a && b\" ."]
        );
        assert_eq!(
            pipeline_stages("grep -rn 'a|b' .").unwrap(),
            ["grep -rn 'a|b' ."]
        );
    }

    #[test]
    fn refuses_to_guess() {
        for cmd in [
            "echo $(grep x .)",
            "echo `grep x .`",
            "cat <<'EOF'\ngrep x\nEOF",
            "grep 'unbalanced",
        ] {
            assert_eq!(chain_segments(cmd), None, "{cmd}");
        }
    }

    #[test]
    fn only_stdout_redirects_count() {
        for cmd in [
            "grep x . > out",
            "grep x . >> out",
            "grep x . 1>out",
            "grep x . &>out",
            "grep x . >&2",
            "grep x . >out 2>&1",
            "grep x . 3>out",
            // Unbalanced: unanalyzable, so fail safe.
            "grep 'x . > out",
        ] {
            assert!(redirects_stdout(cmd), "{cmd}");
        }
        for cmd in [
            "grep x . 2>&1",
            "grep x . 2>&1 | head",
            "grep x . 2>errs",
            "grep x . 2>>errs",
            "grep x . 2>/dev/null",
            "grep x . < in",
            "grep x . | head -5",
            "grep -rn '2>/dev/null' .",
            "grep -rn 'a > b' .",
        ] {
            assert!(!redirects_stdout(cmd), "{cmd}");
        }
    }

    #[test]
    fn command_word_steps_over_noise() {
        assert_eq!(
            command_word("LC_ALL=C sudo /usr/bin/grep -rn x .").unwrap(),
            "grep"
        );
        assert_eq!(command_word("git grep -n x").unwrap(), "grep");
        assert_eq!(command_word("cargo test").unwrap(), "cargo");
    }

    /// `command grep` is the caller's opt-out; it must not read as a search.
    #[test]
    fn command_is_not_stepped_over() {
        assert_eq!(command_word("command grep -rn x .").unwrap(), "command");
        assert!(!is_search("command grep -rn x ."));
    }

    #[test]
    fn search_is_the_producing_stage_only() {
        assert!(is_search("rg -n x src | head"));
        assert!(!is_search("cargo test | grep -E fail"));
    }
}
