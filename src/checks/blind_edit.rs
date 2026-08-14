//! The script that edits a source file by string substitution and never checks
//! the substitution happened. Narrow by construction: the evidence is the whole
//! read-modify-write trio in one interpreter body, not the interpreter itself.

use crate::output::HookOutput;

use crate::checks::marker;

/// Named in the deny, so a script that genuinely wants this can say so. Carries
/// none of the words the credential check refuses, so creating it is not itself
/// a refusal.
pub const WAIVER: &str = "script-edit-waiver";

/// Interpreters whose heredoc body is a program. A shell heredoc is not here: its
/// body is commands, and the checks that read those already run over the head.
/// Perl is absent: its in-place idiom is `perl -i`, which this does not judge.
const INTERPRETERS: [&str; 6] = ["python", "python2", "python3", "ruby", "node", "php"];

/// Reading a whole file into one string. The line-by-line forms are deliberately
/// absent — iterating a file is how an analysis script reads, and those pass.
const SLURPS: [&str; 4] = [
    ".read()",
    "read_text(",
    "file_get_contents(",
    "readFileSync(",
];

/// Substituting into that string. `re.sub` counts: it is the same unverified
/// rewrite in a regex spelling.
const SUBSTITUTES: [&str; 5] = [
    ".replace(",
    "re.sub(",
    ".gsub(",
    "preg_replace(",
    "str_replace(",
];

/// Writing it back. The distinguishing half — a script that reads and substitutes
/// but prints the result is doing analysis, and analysis is not this habit.
const WRITES_BACK: [&str; 6] = [
    "'w'",
    "\"w\"",
    "write_text(",
    "writeFileSync(",
    "file_put_contents(",
    ".write(",
];

fn reason() -> String {
    format!(
        "Blind in-place edit: this script reads a file whole, substitutes into it, and writes it \
         back without checking the match landed. A substitution that matches nothing rewrites nothing \
         and says nothing — the failure is silent, and the file looks edited.\n\n\
         Use the Edit tool: it fails loudly when `old_string` does not match, which is the \
         verification this script does not do. For several edits to one file, several Edit calls; \
         for the same edit throughout, `replace_all`.\n\n\
         If the substitution really is the right tool here — generated output, a file too large to \
         read, a rewrite across many files — take the waiver and re-run:\n  {}\n\
         It is spent by the next such command.",
        marker::command(WAIVER)
    )
}

pub fn check(command: &str) -> Option<HookOutput> {
    if !blind_edit(command) {
        return None;
    }
    // Spent only against a command this would otherwise refuse, so an unrelated
    // call does not consume a waiver the user approved for this one.
    if marker::spend(WAIVER) {
        return None;
    }
    Some(HookOutput::deny("PreToolUse", &reason()))
}

/// The waiver's own creation, forced to a prompt: an allowlisted `touch` must not
/// hand one out unseen.
pub fn waiver_requested(command: &str) -> Option<HookOutput> {
    marker::creation_requested(command, WAIVER).then(|| {
        HookOutput::ask(
            "PreToolUse",
            "Creates a one-shot waiver letting the next script edit a file in place by string \
             substitution, with no check that the match landed.",
        )
    })
}

/// The body of an interpreter heredoc, if this command is one. Everything past
/// the marker line: the marker itself is quoted or not, and the terminator is not
/// worth tracking — text after it is more shell, which contains no script.
fn script_body(command: &str) -> Option<&str> {
    let (head, rest) = command.split_once("<<")?;
    // `python3 - <<'PY'` — the interpreter is the last program word before the
    // redirect, so a `cd x && python3` still reads as python.
    head.split_whitespace()
        .filter_map(|token| {
            token
                .rsplit('/')
                .next()
        })
        .any(|word| INTERPRETERS.contains(&word))
        .then(|| {
            rest.split_once('\n')
                .map(|(_, body)| body)
        })
        .flatten()
}

/// True when the body reads a file whole, substitutes into it, and writes it back.
/// All three, or nothing: any two of them describe an ordinary script.
fn blind_edit(command: &str) -> bool {
    let Some(body) = script_body(command) else {
        return false;
    };
    if !SLURPS
        .iter()
        .any(|needle| body.contains(needle))
    {
        return false;
    }
    if !WRITES_BACK
        .iter()
        .any(|needle| body.contains(needle))
    {
        return false;
    }
    SUBSTITUTES
        .iter()
        .any(|needle| body.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::blind_edit;

    #[test]
    fn catches_the_read_replace_write_trio() {
        for cmd in [
            "python3 - <<'PY'\np='src/main.rs'\ns=open(p).read()\ns = s.replace('a','b')\nopen(p,'w').write(s)\nPY",
            "cd /x && python3 - <<'PY'\ns=open('f.rs').read()\ns=s.replace('x','y')\nopen('f.rs','w').write(s)\nPY",
            "python3 <<PY\nfrom pathlib import Path\nt=Path('a.rs').read_text()\nPath('a.rs').write_text(t.replace('q','r'))\nPY",
            "node - <<'JS'\nlet s=readFileSync('a.ts','utf8')\nwriteFileSync('a.ts', s.replace('x','y'))\nJS",
        ] {
            assert!(blind_edit(cmd), "should deny: {cmd}");
        }
    }

    #[test]
    fn allows_scripts_that_are_not_that_shape() {
        for cmd in [
            // Analysis: iterates the file, writes nothing back.
            "python3 - <<'PY'\nfrom collections import Counter\nc=Counter()\nfor line in open('log'):\n    c[line.split()[0]]+=1\nprint(c)\nPY",
            // Reads whole and substitutes, but prints the result.
            "python3 - <<'PY'\ns=open('f').read()\nprint(s.replace('a','b'))\nPY",
            // Writes a new file with no substitution.
            "python3 - <<'PY'\nopen('out.csv','w').write('a,b\\n')\nPY",
            // A shell heredoc is not a script body.
            "cat <<'EOF' > notes.md\nsome text\nEOF",
            "git commit -F - <<EOF\nfix: a thing\nEOF",
            // No heredoc at all.
            "python3 -c 'print(1)'",
            "rg -n 'open\\(p\\)' src",
        ] {
            assert!(!blind_edit(cmd), "should allow: {cmd}");
        }
    }
}
