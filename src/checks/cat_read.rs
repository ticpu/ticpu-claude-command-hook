//! `cat` standing in for a file read. Judged on where the bytes go, not on the
//! program: the shapes refused are the whole file printed into the transcript and
//! the single file handed to a pipe that could have opened it.

use crate::checks::marker;
use crate::checks::shell;
use crate::output::HookOutput;

/// Named in the deny. Carries none of the words the credential check refuses, so
/// creating it is not itself a refusal.
pub const WAIVER: &str = "cat-read-waiver";

/// Where a judged `cat` sends what it read. The fix differs, so the reason does.
enum Sink {
    /// Last stage with stdout free: the file lands in the transcript.
    Transcript,
    /// One file into a pipe, which the next program can open itself.
    Pipe,
}

fn reason(sink: &Sink) -> String {
    let fix = match sink {
        Sink::Transcript => {
            "Use the Read tool. It numbers the lines, takes an offset and a limit, and records \
             the read that a later Edit is checked against — none of which a printed file \
             carries.\n\n\
             A flag is not refused: `-A`, `-v`, `-T`, `-E`, `-n` ask for bytes no tool result \
             renders, which is the one thing this cannot do."
        }
        Sink::Pipe => {
            "One file into a pipe is a file the next program can open itself: `grep pat file`, \
             `prog < file`. Two or more is the concatenation `cat` exists for and is not \
             refused."
        }
    };
    format!(
        "{fix}\n\n\
         If this is the shape you need — the harness reporting a read you do not hold, output \
         nothing else produces — take the waiver and re-run:\n  {}\n\
         It needs no approval and is spent by the next command this would refuse.",
        marker::command(WAIVER)
    )
}

pub fn check(command: &str) -> Option<HookOutput> {
    let sink = judge(command)?;
    // Spent only against a command this would otherwise refuse, so an unrelated
    // call does not consume it. A credential path never arrives here — that check
    // decides first — so this waiver cannot be the way one is printed.
    if marker::spend(WAIVER) {
        return None;
    }
    Some(HookOutput::deny("PreToolUse", &reason(&sink)))
}

/// The waiver's own creation, allowed outright. Alone among the waivers here: the
/// refusal it answers is over a habit, and the case it cannot settle is the
/// harness's own bookkeeping, which nobody at a prompt can rule on.
pub fn waiver_allowed(command: &str) -> Option<HookOutput> {
    marker::creation_only(command, WAIVER).then(|| {
        HookOutput::allow(
            "PreToolUse",
            "creates the one-shot waiver for the next `cat` of a file (auto-allowed by the hook)",
        )
    })
}

/// The first judged `cat` in the command, if any. A heredoc anywhere makes the
/// command unsplittable and it is left alone, in line with every other check here.
fn judge(command: &str) -> Option<Sink> {
    shell::chain_segments(command)?
        .iter()
        .find_map(|segment| {
            let stages = shell::pipeline_stages(segment)?;
            let last = stages.len() - 1;
            stages
                .iter()
                .enumerate()
                .find_map(|(i, stage)| stage_sink(stage, i == last))
        })
}

fn stage_sink(stage: &str, ends_the_pipeline: bool) -> Option<Sink> {
    if shell::program(stage) != Some("cat") {
        return None;
    }
    let files = plain_cat_files(&shell::program_args(stage)?)?;
    if ends_the_pipeline {
        // `cat a b > merged` writes a file rather than printing one: a copy, and
        // no tool does it.
        if shell::redirects_stdout(stage) {
            return None;
        }
        return (files > 0).then_some(Sink::Transcript);
    }
    (files == 1).then_some(Sink::Pipe)
}

/// How many files this `cat` opens — operands plus an input redirect. `None` when
/// a flag is present: a flag asks for a rendering no tool result gives, which is
/// the use this check has nothing to say about. `-` and `--` are not flags.
fn plain_cat_files(args: &[&str]) -> Option<usize> {
    let mut files = 0;
    let mut expect_target = false;
    for arg in args {
        let token = shell::unquote_token(arg);
        if expect_target {
            expect_target = false;
            continue;
        }
        if token == "-" || token == "--" {
            continue;
        }
        if token.starts_with('-') {
            return None;
        }
        if token.contains('>') {
            expect_target = token.ends_with('>');
            continue;
        }
        if let Some(target) = token.strip_prefix('<') {
            expect_target = target.is_empty();
            files += 1;
            continue;
        }
        files += 1;
    }
    Some(files)
}

#[cfg(test)]
mod tests {
    use super::{Sink, judge};

    fn transcript(command: &str) -> bool {
        matches!(judge(command), Some(Sink::Transcript))
    }

    fn piped(command: &str) -> bool {
        matches!(judge(command), Some(Sink::Pipe))
    }

    #[test]
    fn refuses_a_file_printed_into_the_transcript() {
        for cmd in [
            "cat Makefile",
            "cat src/main.rs src/rules.rs",
            "cd src && cat main.rs",
            "for i in a b; do cat $i; done",
            "sudo cat /etc/hosts",
            "cat < Cargo.toml",
            "/bin/cat Makefile",
        ] {
            assert!(transcript(cmd), "should deny as printed: {cmd}");
        }
    }

    #[test]
    fn refuses_one_file_handed_to_a_pipe() {
        for cmd in [
            "cat Makefile | rg release",
            "cat dump.sql | mysql mydb",
            "cat < f | wc -l",
        ] {
            assert!(piped(cmd), "should deny as piped: {cmd}");
        }
    }

    #[test]
    fn leaves_every_other_shape_alone() {
        for cmd in [
            // A flag asks for what no tool result renders.
            "cat -A Makefile",
            "cat -n src/main.rs",
            "cat --show-all Makefile",
            // Nothing is opened.
            "cat",
            "cat -",
            "rg -n 'cat ' src",
            // Concatenation, which is the command's own job.
            "cat a.log b.log | sort",
            "cat a.log b.log > merged.log",
            "cat one.txt > copy.txt",
            // A heredoc leaves the command unsplittable.
            "cat <<'EOF' > notes.md\nsome text\nEOF",
        ] {
            assert!(judge(cmd).is_none(), "should pass: {cmd}");
        }
    }
}
