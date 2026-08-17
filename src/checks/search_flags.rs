//! Two search flags that do not mean what a grep habit expects them to.

use crate::checks::shell;
use crate::output::HookOutput;

const RG_REPLACE: &str = "`rg -r` is `--replace`, not grep's `--recursive`: `rg -rn PAT dir` prints \
every match rewritten to `n`, and the rewritten text still looks like a normal hit. rg recurses \
by default and `-n` alone numbers the lines. Pass `--replace=` if substitution really is what you \
want.";

const RG_HELP: &str = "`rg -h` is `--help`, not grep's `--no-filename`: it prints usage, exits 0 \
and never searches, so the help text arrives where the matches should be and nothing fails. rg \
spells `--no-filename` `-I`, and drops the path itself once the search names a single file.";

const STREAM_PREFIX: &str = "A search filtering another search's output must not add its own \
`-n`/`-b`/`-H`/`--vimgrep` prefix: it counts the piped stream, so the numbers name no line in any \
file, and the extra prefix hides the path from the fold. Drop the flag — the upstream search \
already prints the real line numbers.";

/// Short flags that swallow the rest of their cluster as a value, so what follows
/// them is not more flags: `rg -trust` is `--type rust`, not `-r ust`.
const RG_VALUE_FLAGS: &[u8] = b"ABCEMTdefgjmrt";
const GREP_VALUE_FLAGS: &[u8] = b"ABCDdefm";

/// Prefixes gf keys on. `-o` reshapes a line too, but it drops the prefix rather
/// than adding one, so the fold degrades to a no-op instead of misreading.
const PREFIX_SHORTS: &[u8] = b"nbH";
const PREFIX_LONGS: [&str; 4] = ["line-number", "byte-offset", "with-filename", "vimgrep"];

/// Long flags taking the next word as their value, so that word is not the
/// pattern. `--regexp` and `--file` are apart: they say where the pattern is.
const LONG_VALUE_FLAGS: &[&str] = &[
    "type",
    "type-not",
    "type-add",
    "glob",
    "iglob",
    "max-count",
    "max-depth",
    "context",
    "after-context",
    "before-context",
    "threads",
    "sort",
    "sortr",
    "replace",
    "color",
    "colors",
    "encoding",
    "engine",
    "binary-files",
    "devices",
    "directories",
    "label",
    "include",
    "exclude",
    "exclude-dir",
];

/// Which of a stage's whitespace-separated words hold the pattern rather than a
/// path: an `-e`/`--regexp` value, or the first positional when no flag claimed
/// it. A search is the one command naming a pattern where the others name a file,
/// so a caller reading paths has to step over these — `rg 'password|secret' src/`
/// names one path, not two. Only the words before the first positional are read,
/// everything past it being a path.
pub fn pattern_words(stage: &str) -> Option<Vec<usize>> {
    let searcher = shell::command_word(stage)?;
    let value_flags = match searcher {
        "rg" => RG_VALUE_FLAGS,
        _ => GREP_VALUE_FLAGS,
    };
    let mut args = shell::program_args(stage)?;
    // `git grep` puts git's own options and the subcommand before the search's.
    if shell::program(stage) != Some(searcher) {
        let at = args
            .iter()
            .position(|word| *word == searcher)?;
        args.drain(..=at);
    }
    let base = stage
        .split_whitespace()
        .count()
        - args.len();
    let mut patterns = Vec::new();
    let mut claimed = false;
    let mut flags_done = false;
    let mut i = 0;
    while let Some(word) = args
        .get(i)
        .copied()
    {
        i += 1;
        if word == "--" {
            flags_done = true;
            continue;
        }
        if let Some(long) = word
            .strip_prefix("--")
            .filter(|_| !flags_done)
        {
            let glued = long.contains('=');
            let name = long
                .split('=')
                .next()
                .unwrap_or(long);
            match name {
                "regexp" | "file" => {
                    claimed = true;
                    if name == "regexp" {
                        patterns.push(base + if glued { i - 1 } else { i });
                    }
                }
                _ if LONG_VALUE_FLAGS.contains(&name) => {}
                _ => continue,
            }
            if !glued {
                i += 1;
            }
            continue;
        }
        let Some(cluster) = word
            .strip_prefix('-')
            .filter(|c| !c.is_empty() && !flags_done)
        else {
            if !claimed {
                patterns.push(base + i - 1);
            }
            break;
        };
        for (n, c) in cluster
            .bytes()
            .enumerate()
        {
            if !c.is_ascii_alphabetic() {
                break;
            }
            if value_flags.contains(&c) {
                let glued = n + 1 < cluster.len();
                if matches!(c, b'e' | b'f') {
                    claimed = true;
                    if c == b'e' {
                        patterns.push(base + if glued { i - 1 } else { i });
                    }
                }
                if !glued {
                    i += 1;
                }
                break;
            }
        }
    }
    Some(patterns)
}

pub fn check(command: &str) -> Option<HookOutput> {
    for segment in shell::chain_segments(command)? {
        let stages = shell::pipeline_stages(segment)?;
        let mut upstream_search = false;
        for stage in stages {
            if !shell::is_searcher(stage) {
                continue;
            }
            let rg = shell::command_word(stage) == Some("rg");
            let flags = Flags::of(stage, if rg { RG_VALUE_FLAGS } else { GREP_VALUE_FLAGS })?;

            if rg && flags.any_short(b"r") {
                return Some(HookOutput::deny("PreToolUse", RG_REPLACE));
            }
            if rg && flags.any_short(b"h") && !asks_only_for_help(stage) {
                return Some(HookOutput::deny("PreToolUse", RG_HELP));
            }
            if upstream_search && (flags.any_short(PREFIX_SHORTS) || flags.any_long(&PREFIX_LONGS))
            {
                return Some(HookOutput::deny("PreToolUse", STREAM_PREFIX));
            }
            upstream_search = true;
        }
    }
    None
}

/// `rg -h` on its own is someone reading the usage. Anything else on the line was
/// meant to search, and the help output replaced it.
fn asks_only_for_help(stage: &str) -> bool {
    shell::program_args(stage).is_some_and(|args| args == ["-h"])
}

struct Flags {
    shorts: Vec<u8>,
    longs: Vec<String>,
}

impl Flags {
    /// The flags a stage actually turns on. Quoted spans are dropped first, so a
    /// pattern naming a flag is never counted as one.
    fn of(stage: &str, value_flags: &[u8]) -> Option<Self> {
        let bare = shell::unquoted(stage)?;
        let mut flags = Flags {
            shorts: Vec::new(),
            longs: Vec::new(),
        };
        let mut words = bare.split_whitespace();
        while let Some(word) = words.next() {
            if word == "--" {
                break;
            }
            if let Some(long) = word.strip_prefix("--") {
                flags
                    .longs
                    .push(
                        long.split('=')
                            .next()
                            .unwrap_or(long)
                            .to_string(),
                    );
                continue;
            }
            let Some(cluster) = word
                .strip_prefix('-')
                .filter(|c| !c.is_empty())
            else {
                continue;
            };
            for (i, c) in cluster
                .bytes()
                .enumerate()
            {
                // A digit ends the cluster: `-B6` is a value, `-5` is grep's context.
                if !c.is_ascii_alphabetic() {
                    break;
                }
                flags
                    .shorts
                    .push(c);
                if value_flags.contains(&c) {
                    // Nothing glued on means the value is the next word, which must
                    // not then be read as flags of its own (`-e -n`).
                    if i + 1 == cluster.len() {
                        words.next();
                    }
                    break;
                }
            }
        }
        Some(flags)
    }

    fn any_short(&self, wanted: &[u8]) -> bool {
        self.shorts
            .iter()
            .any(|c| wanted.contains(c))
    }

    fn any_long(&self, wanted: &[&str]) -> bool {
        self.longs
            .iter()
            .any(|l| wanted.contains(&l.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::{check, pattern_words};

    #[test]
    fn the_pattern_is_told_from_the_paths() {
        for (cmd, want) in [
            ("rg foo src", vec![1]),
            ("rg -n foo src", vec![2]),
            ("rg -tyaml -n foo src", vec![3]),
            ("rg --type yaml foo src", vec![3]),
            ("rg -- -foo src", vec![2]),
            ("git grep -n foo -- src", vec![3]),
            ("sudo rg -n foo src", vec![3]),
            // The flag says where the pattern is, so no positional is one.
            ("rg -e foo src", vec![2]),
            ("rg -efoo src", vec![1]),
            ("rg --regexp=foo src", vec![1]),
            ("rg --regexp foo src", vec![2]),
            // A pattern file is a path, and it leaves no positional pattern.
            ("rg -f pats.txt src", vec![]),
            ("rg --file pats.txt src", vec![]),
        ] {
            assert_eq!(pattern_words(cmd), Some(want), "{cmd}");
        }
    }

    fn denied(command: &str) -> bool {
        check(command).is_some()
    }

    #[test]
    fn denies_every_shape_of_rg_dash_r() {
        for cmd in [
            "rg -rn foo src",
            "rg -nrl foo src",
            "rg -r foo src",
            "rg -r -n foo src",
            "rg -in 'a|b' src | rg -r x",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn leaves_rg_flags_that_are_not_replace_alone() {
        for cmd in [
            "rg -n foo src",
            "rg -trust -n foo",
            "rg -tr -n foo",
            "rg --replace='$1' foo src",
            "rg -n -- -r src",
            // `-r` inside the pattern is not a flag.
            "rg -n '\\-rn' src",
            // grep's own -r is recursion.
            "grep -rn foo src",
            "ugrep -rn foo src",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn denies_rg_dash_h_used_as_no_filename() {
        for cmd in [
            "rg -ohN '\\-j \\+?\\w+' .",
            "rg -h foo src",
            "rg -oh foo .",
            "rg --no-heading -h foo .",
            "rg -n foo src | rg -h bar",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn leaves_a_real_help_request_and_grep_alone() {
        for cmd in [
            "rg -h",
            "rg -h | head -40",
            "rg --help",
            // grep's -h really is --no-filename.
            "grep -rh foo src",
            // -t takes the rest of the cluster as its value.
            "rg -th -n foo",
            // -e with nothing glued on takes the next word as the pattern.
            "rg -e -h foo",
            "rg -n '\\-h' src",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn denies_a_stream_prefix_on_a_filtering_search() {
        for cmd in [
            "rg -n foo src | rg -n bar",
            "grep -rn foo src | grep -in bar",
            "rg -n foo src | rg -v x | rg --vimgrep bar",
            "rg -n foo src | rg -H bar",
            "ls /x; grep -rn a . | grep -bn b",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn a_filter_without_a_prefix_flag_is_fine() {
        for cmd in [
            "rg -n foo src | rg bar",
            "rg -n foo src | rg -v bar | head",
            // The numbers here are the file's, since nothing upstream searched.
            "cat x | grep -n foo",
            "cargo test 2>&1 | grep -n fail",
            // A pattern that merely contains the flag.
            "rg -n foo src | rg -- '-n'",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }
}
