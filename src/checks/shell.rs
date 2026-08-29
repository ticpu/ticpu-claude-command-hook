//! Just enough shell parsing to tell a plain pipeline from everything else.
//! Quote-aware — quoted spans and command substitutions are masked out, so an
//! operator inside either is never read as one — and it refuses to guess: a
//! heredoc or an unbalanced quote comes back as `None` so callers fail open
//! instead of mis-splitting.

use std::ops::Range;
use std::str::SplitWhitespace;

/// Commands that read files and print `path:line:` — the ones worth folding.
const SEARCHERS: [&str; 7] = ["grep", "egrep", "fgrep", "rgrep", "ugrep", "ug", "rg"];

/// Wrappers that take the real command as their arguments. `command` is
/// deliberately absent: writing `command grep` is how a caller asks for the real
/// binary and to be left alone, so it must not classify as a search.
const WRAPPERS: [&str; 6] = ["env", "sudo", "time", "nice", "stdbuf", "timeout"];

/// Wrappers whose arguments include a bare value of their own before the command:
/// `timeout 45 ssh …` runs ssh, not `45`.
const WRAPPER_VALUES: [(&str, usize); 1] = [("timeout", 1)];

/// Later pipeline stages that only display what they receive. Anything else may
/// parse the path off each line — folding would feed it truncated paths — or,
/// for the read-only classifier, run arbitrary code on the piped output.
/// `wc` would count folded lines, and `sort -o <file>` writes a file.
const DISPLAY_ONLY: [&str; 5] = ["head", "tail", "less", "cat", "nl"];

/// Commands that read and print and cannot write a file or run a program. Used
/// where one decision covers a whole chain: such a segment can ride along on an
/// auto-allow without widening what it permits.
const READ_ONLY_UTILS: [&str; 13] = [
    "ls", "pwd", "file", "stat", "wc", "basename", "dirname", "realpath", "readlink", "date",
    "uname", "id", "printf",
];

/// Tokens that introduce a command without being one. Stepping over them is what
/// lets a loop body or a brace group be classified as the command it runs.
const GROUPING: [&str; 6] = ["{", "(", "!", "do", "then", "else"];

/// git's own global options. The value-taking ones must not have their value
/// mistaken for the subcommand.
const GIT_VALUE_OPTIONS: [&str; 5] = ["-C", "-c", "--git-dir", "--work-tree", "--namespace"];

/// Top-level segments paired with the operator that follows each one (`""` for
/// the last), so a caller can rewrite one segment and rebuild the command.
pub fn chain_parts(command: &str) -> Option<Vec<(&str, &str)>> {
    split(command, |b, i| match b[i] {
        b';' | b'\n' => Some(1),
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

/// Any redirect at all. `redirects_stdout` deliberately lets `2>` through — for the
/// fold that is right, since gf still sees stdout — but an allow turns on whether the
/// command can touch a file, and `2>file` truncates it.
pub fn redirects_anything(segment: &str) -> bool {
    redirects_stdout(segment) || unquoted(segment).is_none_or(|bare| bare.contains('>'))
}

/// `>>` is one operator; its second byte must not count as another redirect.
fn preceded_by(b: &[u8], i: usize, c: u8) -> bool {
    i > 0 && b[i - 1] == c
}

/// A bare `2` glued to the `>`. A longer number is a higher fd, deliberately left
/// on the stdout side: fail-safe beats guessing what an exotic fd does.
fn stderr_fd(b: &[u8], i: usize) -> bool {
    preceded_by(b, i, b'2') && !(i > 1 && b[i - 2].is_ascii_digit())
}

/// The program a stage runs, by basename, with no subcommand lookup: `sudo git
/// commit` and `/usr/bin/git commit` both come back as `git`. `VAR=v`
/// assignments, grouping punctuation and wrappers are stepped over, and a
/// wrapper's own options take the following token as a possible value —
/// `sudo -u postgres psql` runs psql. A value-less wrapper flag therefore eats
/// the program name and the stage comes back unclassified, which is the safe way
/// to be wrong here.
pub fn program(stage: &str) -> Option<&str> {
    program_and_args(stage).map(|(program, _)| program)
}

/// The command word of a stage: as `program`, except `git` yields its subcommand
/// so `git grep` classifies as a search. Global options are stepped over first,
/// so `git -C /x grep` is still a grep.
pub fn command_word(stage: &str) -> Option<&str> {
    let (program, args) = program_and_args(stage)?;
    if program == "git" {
        return git_subcommand(args);
    }
    Some(program)
}

/// The tokens after a stage's program word, wrappers and their options already
/// stepped over. Whitespace-split, so a quoted argument holding a space arrives
/// as several tokens.
pub fn program_args(stage: &str) -> Option<Vec<&str>> {
    program_and_args(stage).map(|(_, args)| args.collect())
}

fn program_and_args(stage: &str) -> Option<(&str, SplitWhitespace<'_>)> {
    let mut words = stage.split_whitespace();
    let mut in_wrapper_options = false;
    let mut wrapper_values = 0;
    while let Some(raw) = words.next() {
        let word = basename(raw);
        if GROUPING.contains(&word) {
            continue;
        }
        if WRAPPERS.contains(&word) {
            in_wrapper_options = true;
            wrapper_values += WRAPPER_VALUES
                .iter()
                .find(|(name, _)| *name == word)
                .map_or(0, |(_, count)| *count);
            continue;
        }
        if word.contains('=') && !word.starts_with('-') {
            continue;
        }
        if in_wrapper_options && raw.starts_with('-') {
            if !raw.contains('=') {
                let _ = words.next();
            }
            continue;
        }
        if wrapper_values > 0 {
            wrapper_values -= 1;
            continue;
        }
        return Some((word, words));
    }
    None
}

/// The first token past git's global options — the subcommand, or `None` when the
/// invocation is only global options.
fn git_subcommand<'a>(mut args: SplitWhitespace<'a>) -> Option<&'a str> {
    while let Some(token) = args.next() {
        if GIT_VALUE_OPTIONS.contains(&token) {
            let _ = args.next();
            continue;
        }
        if token.starts_with('-') {
            continue;
        }
        return Some(basename(token));
    }
    None
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

/// Search options that hand the searcher a program of its own — a preprocessor, a
/// pager, an external command. An allow in front of one is an allow in front of
/// whatever it names.
const SEARCH_RUNS_A_PROGRAM: [&str; 5] = [
    "--pre",
    "--hostname-bin",
    "--filter",
    "--open-files-in-pager",
    "--ext-cmd",
];

pub fn search_runs_a_program(word: &str) -> bool {
    let name = word
        .split('=')
        .next()
        .unwrap_or(word);
    SEARCH_RUNS_A_PROGRAM.contains(&name) || word.starts_with("-O")
}

/// A search that prints its matches and does nothing else, so it adds no side
/// effect to whatever feeds it.
pub fn is_plain_search(stage: &str) -> bool {
    is_searcher(stage)
        && !stage
            .split_whitespace()
            .any(search_runs_a_program)
}

/// An `echo` labelling the output of the commands around it: it runs nothing and
/// writes nothing, so it does not count as company in a chain. Only as a lone
/// stage with no redirect — `echo x > f` writes a file and `echo x | sh` runs one —
/// and only without a substitution, which runs whatever it names before echo sees it.
pub fn is_lone_echo(segment: &str) -> bool {
    !redirects_stdout(segment)
        && substitution_spans(segment).is_some_and(|spans| spans.is_empty())
        && pipeline_stages(segment)
            .is_some_and(|stages| stages.len() == 1 && command_word(segment) == Some("echo"))
}

/// The directory a `cd <path>` segment moves to, when that is all it does. A flag,
/// a bare `cd` (to `$HOME`), `cd -`, or a redirect glued to the path (`cd /x>y`
/// truncates `y`) all disqualify it.
pub fn bare_cd_target(segment: &str) -> Option<&str> {
    let mut words = segment.split_whitespace();
    if words.next() != Some("cd") {
        return None;
    }
    let path = words.next()?;
    (words
        .next()
        .is_none()
        && !path.starts_with('-')
        && !redirects_stdout(segment))
    .then_some(path)
}

pub fn is_bare_cd(segment: &str) -> bool {
    bare_cd_target(segment).is_some()
}

/// A segment that only names a variable: one stage, every word a `NAME=value`,
/// no redirect. It runs nothing, so it rides along on an auto-allow — a value
/// that *would* run something is a substitution, refused ahead of every allow.
pub fn is_bare_assignment(segment: &str) -> bool {
    !redirects_anything(segment)
        && pipeline_stages(segment).is_some_and(|stages| stages.len() == 1)
        && !segment
            .trim()
            .is_empty()
        && segment
            .split_whitespace()
            .all(is_assignment_word)
}

/// Stricter than the prefix `program_and_args` steps over: that one only has to
/// skip a word, this one grants an allow, so the name has to be one.
fn is_assignment_word(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A segment that only reads and prints: every stage is a utility that cannot
/// write a file or run a program, and stdout goes nowhere but the terminal.
pub fn is_read_only_util(segment: &str) -> bool {
    !redirects_stdout(segment)
        && pipeline_stages(segment).is_some_and(|stages| {
            stages
                .iter()
                .all(|stage| {
                    command_word(stage).is_some_and(|word| {
                        READ_ONLY_UTILS.contains(&word)
                            || DISPLAY_ONLY.contains(&word)
                            || word == "echo"
                    })
                })
        })
}

/// The stage's own first word, by basename, with only a `VAR=v` prefix stepped
/// over. Deliberately not `program`, which reads through a wrapper: a wrapper is
/// something to see through when deciding what to refuse and something to refuse
/// when deciding what to grant, since the allow would cover it too.
pub fn leading_word(stage: &str) -> Option<&str> {
    stage
        .split_whitespace()
        .find(|word| word.starts_with('-') || !word.contains('='))
        .map(basename)
}

/// True when a pipeline stage only displays what it is handed.
pub fn is_display_only(stage: &str) -> bool {
    command_word(stage).is_some_and(|w| DISPLAY_ONLY.contains(&w))
}

/// A later stage that writes nothing and runs nothing. Weaker than `grep_fold`'s
/// display-only test, which additionally has to survive gf's folding — here the
/// only question is whether the stage adds a side effect to the producer's.
pub fn is_harmless_consumer(stage: &str) -> bool {
    if is_display_only(stage) || is_plain_search(stage) {
        return true;
    }
    match command_word(stage) {
        Some("wc") => true,
        Some("sed") => prints_line_ranges(stage),
        _ => false,
    }
}

/// `sed` restricted to selecting lines: every argument is the quiet flag, a bare
/// `-e`, or a script built only from line numbers and `p`/`q`/`d`. That leaves out
/// `-i`, an `s///w` or `w` command and GNU's `e`, so nothing is written or run. A
/// glued `-e<script>`/`--expression=` is not accepted, since the script would ride
/// along unchecked.
fn prints_line_ranges(stage: &str) -> bool {
    let mut args = stage.split_whitespace();
    let _ = args.next(); // "sed"
    let mut scripts = 0;
    for arg in args {
        if matches!(arg, "-n" | "--quiet" | "--silent" | "-e") {
            continue;
        }
        let script = unquote_token(arg);
        if script.is_empty()
            || !script
                .chars()
                .all(|c| c.is_ascii_digit() || ",;p$qd".contains(c))
        {
            return false;
        }
        scripts += 1;
    }
    scripts > 0
}

/// Everything before a heredoc marker. Past it is data — a commit message, a SQL
/// body — not options, so a check reading flags must stop here.
pub fn before_heredoc(cmd: &str) -> &str {
    cmd.split("<<")
        .next()
        .unwrap_or(cmd)
}

/// The text in front of a heredoc whose body cannot act: one `<<` with a quoted
/// delimiter, terminated, with nothing after the terminator line. A quoted
/// delimiter is what makes the body literal — no expansion, no substitution — so
/// the head alone decides what runs, and it is ordinary command text every other
/// function here can scan. `None` on an unquoted delimiter, a herestring, a
/// missing terminator, or a command resuming past it.
pub fn inert_heredoc(cmd: &str) -> Option<&str> {
    let start = cmd.find("<<")?;
    let head = &cmd[..start];
    // The `<<` has to be shell syntax and not text: a quote opened in the head and
    // closed in the body leaves the head unbalanced, which `scan` refuses.
    if !analyzable(head) || unquoted_mask(head).is_none() {
        return None;
    }
    let rest = &cmd[start + 2..];
    if rest.starts_with('<') {
        return None;
    }
    let (strip_tabs, rest) = match rest.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let (line, body) = rest
        .trim_start_matches(' ')
        .split_once('\n')?;
    let quote = line
        .chars()
        .next()
        .filter(|c| *c == '\'' || *c == '"')?;
    let end = line[1..].find(quote)? + 1;
    let delimiter = &line[1..end];
    if delimiter.is_empty()
        || !delimiter
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !line[end + 1..]
            .trim()
            .is_empty()
    {
        return None;
    }
    let mut consumed = 0;
    let mut terminated = false;
    for raw in body.split_inclusive('\n') {
        consumed += raw.len();
        let text = raw.trim_end_matches('\n');
        let text = if strip_tabs {
            text.trim_start_matches('\t')
        } else {
            text
        };
        if text == delimiter {
            terminated = true;
            break;
        }
    }
    (terminated
        && body[consumed..]
            .trim()
            .is_empty())
    .then_some(head)
}

/// A token in command position: first, or right after a chain operator or pipe.
/// For use on text the splitters gave up on, so glued forms (`cd /x; git …`) count.
pub fn starts_a_command(tokens: &[&str], i: usize) -> bool {
    i == 0 || tokens[i - 1].ends_with(['&', ';', '|'])
}

/// One token with its surrounding quotes dropped — `"."` reaches a program as the
/// same argument the bare form does. Not `unquoted`, which deletes the quoted span
/// content and all: a token that *is* a quoted path has to survive whole.
pub fn unquote_token(tok: &str) -> &str {
    tok.strip_prefix(['"', '\''])
        .and_then(|s| s.strip_suffix(['"', '\'']))
        .unwrap_or(tok)
}

fn basename(word: &str) -> &str {
    word.rsplit('/')
        .next()
        .unwrap_or(word)
}

/// A heredoc is the one shape left that cannot be masked: its body is arbitrary
/// text whose end this does not track.
fn analyzable(command: &str) -> bool {
    !command.contains("<<")
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

/// Marks the bytes that are outside quotes and substitutions and are not escapes
/// — the only ones that can be shell operators.
fn unquoted_mask(s: &str) -> Option<Vec<bool>> {
    Some(scan(s)?.outside)
}

/// Whether a command runs anything through `$( )` or backticks — including a command
/// that cannot be scanned at all, which has to count as one. What a substitution runs
/// is decided before the surrounding program is reached and is not judged by anything
/// that reads the surrounding program, so no allow may cover a command carrying one.
pub fn has_substitution(command: &str) -> bool {
    substitution_spans(command).is_none_or(|spans| !spans.is_empty())
}

/// Every outermost `$( )` or backtick span, markers included, so a caller can lift
/// one out of the text around it and read what is left.
pub fn substitution_spans(s: &str) -> Option<Vec<Range<usize>>> {
    Some(scan(s)?.substitutions)
}

/// Every outermost quoted span, markers included. A body handed to a shell arrives
/// as one of these, so a caller judging what the far end runs reads it from here
/// rather than from `program_args`, which splits it on whitespace.
pub fn quoted_spans(s: &str) -> Option<Vec<Range<usize>>> {
    Some(scan(s)?.quoted)
}

struct Scan {
    outside: Vec<bool>,
    substitutions: Vec<Range<usize>>,
    quoted: Vec<Range<usize>>,
}

/// Sole quote parser in this module. An unterminated quote or `$(` means the shape
/// cannot be trusted, so `None`.
fn scan(s: &str) -> Option<Scan> {
    let b = s.as_bytes();
    let mut outside = vec![false; b.len()];
    let mut substitutions = Vec::new();
    let mut quoted = Vec::new();
    let mut opened = 0usize;
    let mut quote_opened = 0usize;
    let mut i = 0;
    let mut quote: Option<u8> = None;
    let mut depth = 0usize;
    let mut backtick = false;
    // The quote each substitution opened inside, restored when it closes: text
    // within `$( )` is parsed as a command however the substitution was reached.
    let mut enclosing: Vec<Option<u8>> = Vec::new();
    while i < b.len() {
        match quote {
            // Single quotes take everything literally, including backslashes.
            Some(q) => {
                if b[i] == b'\\' && q == b'"' {
                    i += 2;
                    continue;
                }
                // A double quote does not stop a substitution — `"$(cmd)"` runs it.
                if q == b'"' && b[i] == b'$' && b.get(i + 1) == Some(&b'(') {
                    if depth == 0 && !backtick {
                        opened = i;
                    }
                    depth += 1;
                    enclosing.push(quote);
                    quote = None;
                    i += 2;
                    continue;
                }
                if b[i] == q {
                    quote = None;
                    if depth == 0 && !backtick {
                        quoted.push(quote_opened..i + 1);
                    }
                }
                i += 1;
            }
            None => match b[i] {
                b'\\' => i += 2,
                b'\'' | b'"' => {
                    quote = Some(b[i]);
                    if depth == 0 && !backtick {
                        quote_opened = i;
                    }
                    i += 1;
                }
                b'`' => {
                    backtick = !backtick;
                    if depth == 0 {
                        if backtick {
                            opened = i;
                        } else {
                            substitutions.push(opened..i + 1);
                        }
                    }
                    i += 1;
                }
                b'$' if b.get(i + 1) == Some(&b'(') => {
                    if depth == 0 && !backtick {
                        opened = i;
                    }
                    depth += 1;
                    enclosing.push(None);
                    i += 2;
                }
                b')' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 && !backtick {
                        substitutions.push(opened..i + 1);
                    }
                    quote = enclosing
                        .pop()
                        .flatten();
                    i += 1;
                }
                _ => {
                    outside[i] = depth == 0 && !backtick;
                    i += 1;
                }
            },
        }
    }
    (quote.is_none() && depth == 0 && !backtick).then_some(Scan {
        outside,
        substitutions,
        quoted,
    })
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
    // A command may end on its separator — a trailing `;` or newline is normal,
    // not the malformed input the emptiness check below is there to reject.
    if parts.len() > 1
        && parts
            .last()
            .is_some_and(|(part, _)| part.is_empty())
    {
        parts.pop();
    }
    parts
        .iter()
        .all(|(part, _)| !part.is_empty())
        .then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inert_heredoc_is_the_quoted_terminated_one() {
        assert_eq!(
            inert_heredoc("git commit -F - <<'EOF'\nmsg\nEOF\n"),
            Some("git commit -F - ")
        );
        assert_eq!(inert_heredoc("cat <<-\"E\"\n\tmsg\n\tE"), Some("cat "));
        // Expanded body, herestring, missing terminator, and a command past it.
        assert_eq!(inert_heredoc("cat <<EOF\nmsg\nEOF"), None);
        assert_eq!(inert_heredoc("cat <<<'msg'"), None);
        assert_eq!(inert_heredoc("cat <<'EOF'\nmsg\n"), None);
        assert_eq!(inert_heredoc("cat <<'EOF'\nmsg\nEOF\nrm -rf /x"), None);
        // The marker sits inside a quote the body closes: not shell syntax.
        assert_eq!(inert_heredoc("echo 'a << b'\n"), None);
    }

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
            "cat <<'EOF'\ngrep x\nEOF",
            "grep 'unbalanced",
            "echo $(grep x .",
            "echo `grep x .",
        ] {
            assert_eq!(chain_segments(cmd), None, "{cmd}");
        }
    }

    /// A substitution is masked, not refused: the operators inside it belong to
    /// the inner command, and the outer one is still a single segment.
    #[test]
    fn substitutions_are_masked() {
        assert_eq!(
            chain_segments("echo $(ls; pwd)").unwrap(),
            ["echo $(ls; pwd)"]
        );
        assert_eq!(
            chain_segments("echo `ls; pwd`").unwrap(),
            ["echo `ls; pwd`"]
        );
        assert_eq!(
            chain_segments("grep -rn foo $(pwd) && ls").unwrap(),
            ["grep -rn foo $(pwd)", "ls"]
        );
        assert_eq!(
            unquoted("grep -rn foo $(pwd) 2>/dev/null").unwrap(),
            "grep -rn foo  2>/dev/null"
        );
    }

    #[test]
    fn substitution_spans_cover_the_markers_and_survive_quoting() {
        let cmd = "mongosh \"$(yq -r '.uri' f.yaml)\" --eval 'db.x.count()'";
        let spans = substitution_spans(cmd).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(&cmd[spans[0].clone()], "$(yq -r '.uri' f.yaml)");

        let cmd = "URI=`yq -r .uri f.yaml`";
        let spans = substitution_spans(cmd).unwrap();
        assert_eq!(&cmd[spans[0].clone()], "`yq -r .uri f.yaml`");

        // Literal inside single quotes: nothing is substituted, nothing is a span.
        assert!(
            substitution_spans("rg '\\$\\(cat f\\)' notes.md")
                .unwrap()
                .is_empty()
        );
        assert!(substitution_spans("echo \"$(ls").is_none());
    }

    /// A newline separates two commands as surely as `;`, and a command may end
    /// on its separator.
    #[test]
    fn newlines_separate_and_trailing_ones_do_not() {
        assert_eq!(
            chain_segments("cargo build\ngit status").unwrap(),
            ["cargo build", "git status"]
        );
        assert_eq!(chain_segments("ls /x\n").unwrap(), ["ls /x"]);
        assert_eq!(chain_segments("ls /x;").unwrap(), ["ls /x"]);
        // A continuation is one command: the escape takes the newline with it.
        assert_eq!(
            chain_segments("psql \\\n  -c 'select 1'").unwrap(),
            ["psql \\\n  -c 'select 1'"]
        );
    }

    #[test]
    fn program_sees_through_paths_wrappers_and_groups() {
        for stage in [
            "git commit -m x",
            "/usr/bin/git commit -m x",
            "sudo git commit -m x",
            "env git commit -m x",
            "{ git commit -m x",
            "do git commit -m x",
        ] {
            assert_eq!(program(stage), Some("git"), "{stage}");
        }
        assert_eq!(program("cargo test"), Some("cargo"));
    }

    /// A global option must not have its value read as the subcommand.
    #[test]
    fn git_global_options_do_not_hide_the_subcommand() {
        for stage in [
            "git grep -n foo",
            "git -C /x grep -n foo",
            "git -C=/x grep -n foo",
            "git --no-pager grep -n foo",
            "git -c core.pager=less grep -n foo",
        ] {
            assert_eq!(command_word(stage), Some("grep"), "{stage}");
        }
        assert!(is_search("do grep -n foo x"));
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
        assert_eq!(command_word("sudo -u postgres psql -c x").unwrap(), "psql");
        assert_eq!(command_word("sudo apt install mysql").unwrap(), "apt");
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
