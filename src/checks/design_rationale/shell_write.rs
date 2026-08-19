//! The route around the gate. The reviews below hang off the `Edit` and `Write`
//! tools, so a shell command that rewrites the same document is judged by nobody —
//! it is refused here and told which tool the gate is on.

use crate::checks::marker;
use crate::checks::shell;
use crate::output::HookOutput;

pub const WAIVER: &str = "design-rationale-shell-write";

const DOCUMENT: &str = "design-rationale.md";

/// Programs that leave a file they name holding something else. An interpreter is
/// here for being able to write anything at all; a searcher, a pager and a checksum
/// are absent because reading this document has to stay free.
const MUTATORS: &[&str] = &[
    "mv", "install", "dd", "ln", "tee", "truncate", "touch", "patch", "rm", "shred", "sponge",
    "ed", "vi", "vim", "nvim", "nano", "emacs", "python", "python2", "python3", "ruby", "node",
    "php", "perl",
];

/// Mutators that only rewrite their *last* argument. Reading the document into a
/// copy is not a write of it.
const DESTINATION_ONLY: &[&str] = &["cp", "install"];

/// git subcommands that replace a working-tree file. `commit` is deliberately
/// absent: a message naming this document does not touch it, and the whole point of
/// this check is that a commit describing an edit is not itself one.
const REPLACES_FILES: &[&str] = &[
    "checkout", "restore", "apply", "am", "mv", "rm", "clean", "stash",
];

fn reason() -> String {
    format!(
        "This rewrites a {DOCUMENT} from the shell. That document is reviewed on Edit and Write — \
         the countable rules, then a judged read of the passage — and a shell write reaches \
         neither, so whatever it adds lands unread.\n\n\
         Use the Edit tool: it is the only route the gate is on, and its `old_string` mismatch is \
         also the check a read-modify-write pipeline does not do.\n\n\
         If the shell really is the tool here — a revert, a rename, a file generated whole — take \
         the waiver and re-run:\n  {}\n\
         It is spent by the next command this would refuse.",
        marker::command(WAIVER)
    )
}

pub fn check(command: &str) -> Option<HookOutput> {
    if !rewrites_document(command) {
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
            "Creates a one-shot pass letting the next shell command rewrite a design-rationale.md \
             without the review an Edit of it goes through.",
        )
    })
}

/// A command that can leave a `design-rationale.md` different from how it found it.
/// A command holding a heredoc cannot be split into stages, so its text is judged
/// whole instead of waved through: the body of an interpreter heredoc is a program
/// that can write the document, and the marker line can carry the redirect that does.
fn rewrites_document(command: &str) -> bool {
    if !command.contains(DOCUMENT) {
        return false;
    }
    match shell::chain_segments(command) {
        Some(segments) => segments
            .iter()
            .flat_map(|segment| shell::pipeline_stages(segment).unwrap_or_else(|| vec![segment]))
            .any(writes_document),
        None => writes_document(command),
    }
}

/// Either the document is where this stage's output is sent, or the program is one
/// that rewrites what it names.
fn writes_document(stage: &str) -> bool {
    if redirected_into(stage) {
        return true;
    }
    match shell::program(stage) {
        Some("git") => {
            shell::command_word(stage).is_some_and(|sub| REPLACES_FILES.contains(&sub))
                && names_document(stage)
        }
        // In-place is the whole difference: without it both read and print.
        Some("sed") | Some("awk") => in_place(stage) && names_document(stage),
        Some(program) if DESTINATION_ONLY.contains(&program) => shell::program_args(stage)
            .and_then(|args| {
                args.iter()
                    .rev()
                    .find(|arg| !arg.starts_with('-'))
                    .copied()
            })
            .is_some_and(is_document),
        Some(program) => MUTATORS.contains(&program) && names_document(stage),
        None => false,
    }
}

/// The document as a redirect target. Textual, so a write spelled inside quotes —
/// an awk action, a shell line in a heredoc body — counts the same as a bare one.
fn redirected_into(stage: &str) -> bool {
    stage
        .split('>')
        .skip(1)
        .any(|rest| {
            rest.trim_start_matches('>')
                .split_whitespace()
                .next()
                .is_some_and(is_document)
        })
}

fn names_document(stage: &str) -> bool {
    stage
        .split_whitespace()
        .any(is_document)
}

/// A token naming the document, whatever the surrounding syntax left on it. The
/// quoted pieces are tested one by one, since inside an interpreter body the name is
/// a string literal in the middle of an expression rather than a word of its own.
fn is_document(token: &str) -> bool {
    token
        .split(['"', '\''])
        .any(|piece| {
            piece
                .trim_end_matches(['}', ')', ';', ','])
                .ends_with(DOCUMENT)
        })
}

/// `-i`, `--in-place`, a glued suffix (`-i.bak`) or a cluster carrying it (`-ni`).
fn in_place(stage: &str) -> bool {
    shell::program_args(stage).is_some_and(|args| {
        args.iter()
            .any(|arg| match arg.strip_prefix("--") {
                Some(long) => long.starts_with("in-place"),
                None => arg
                    .strip_prefix('-')
                    .is_some_and(|short| short.contains('i')),
            })
    })
}

#[cfg(test)]
mod tests {
    use super::rewrites_document;

    #[test]
    fn catches_a_shell_write_of_the_document() {
        for command in [
            // The awk/mv round trip this check exists for.
            "awk 'NR==FNR{next}1' new.md docs/design-rationale.md > dr.md && mv dr.md docs/design-rationale.md",
            "cat >> docs/design-rationale.md <<'EOF'\n## A section\nEOF",
            "cat > docs/design-rationale.md <<'EOF'\ntext\nEOF",
            "sed -i 's/a/b/' docs/design-rationale.md",
            "sed -i.bak '1d' ../other/docs/design-rationale.md",
            "python3 - <<'PY'\nopen('docs/design-rationale.md','w').write(s)\nPY",
            "printf '%s' \"$SECTION\" >> \"$W/docs/design-rationale.md\"",
            "tee -a docs/design-rationale.md",
            "git checkout HEAD~1 -- docs/design-rationale.md",
            "cp scratch/draft.md docs/design-rationale.md",
            "mv docs/design-rationale.md docs/old.md",
            "truncate -s 0 docs/design-rationale.md",
            "awk -i inplace '{print}' docs/design-rationale.md",
            "awk '{print > \"docs/design-rationale.md\"}' draft.md",
        ] {
            assert!(rewrites_document(command), "should deny: {command}");
        }
    }

    #[test]
    fn reading_or_naming_the_document_is_not_writing_it() {
        for command in [
            "cat docs/design-rationale.md",
            "rg -n 'codec' docs/design-rationale.md",
            "sed -n '1,40p' docs/design-rationale.md",
            "wc -l docs/design-rationale.md",
            "git diff docs/design-rationale.md",
            "git log --oneline -- docs/design-rationale.md",
            "git show HEAD:docs/design-rationale.md",
            "git commit -m \"docs: fold the retry note into design-rationale.md\"",
            "git commit -F - <<EOF\ndocs: rewrite a design-rationale.md section\nEOF",
            "echo 'see docs/design-rationale.md' > scratch/notes.md",
            "cp docs/design-rationale.md scratch/backup.md",
            "diff -u docs/design-rationale.md scratch/draft.md",
            "ls -l docs/design-rationale.md",
            // Another document entirely.
            "sed -i 's/a/b/' docs/loopback-bowout.md",
        ] {
            assert!(!rewrites_document(command), "should allow: {command}");
        }
    }
}
