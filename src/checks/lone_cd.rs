use crate::checks::shell;
use crate::output::HookOutput;

const LONE_CD: &str = "A `cd` on its own does nothing: every Bash call runs in a fresh shell, so \
the directory is gone before the next call reads it. Put the `cd` in the same command as the work \
(`cd /x && <cmd>`), or give the command an absolute path.";

/// Denies a command that only changes directory. Nothing observes the change, so
/// the call is pure noise — usually a retry after a chained `cd` was refused,
/// which splitting cannot fix.
pub fn check(command: &str) -> Option<HookOutput> {
    let segments = shell::chain_segments(command)?;
    (!segments.is_empty()
        && segments
            .iter()
            .all(|segment| is_only_cd(segment)))
    .then(|| HookOutput::deny("PreToolUse", LONE_CD))
}

/// A single `cd` stage with no stdout redirect: `cd /x > f` and `cd /x | tee f`
/// leave something behind, so they are not the no-op this denies.
fn is_only_cd(segment: &str) -> bool {
    !shell::redirects_stdout(segment)
        && shell::pipeline_stages(segment).is_some_and(|stages| {
            stages.len() == 1
                && segment
                    .split_whitespace()
                    .next()
                    == Some("cd")
        })
}

#[cfg(test)]
mod tests {
    use super::check;

    fn denied(command: &str) -> bool {
        check(command).is_some()
    }

    #[test]
    fn denies_a_command_that_only_changes_directory() {
        for cmd in [
            "cd /x",
            "cd ~/GIT/eido",
            "cd",
            "cd -",
            "cd /x && cd /y",
            "cd /x; cd /y",
        ] {
            assert!(denied(cmd), "{cmd}");
        }
    }

    #[test]
    fn leaves_a_cd_that_leads_somewhere_alone() {
        for cmd in [
            "cd /x && git status",
            "cd /x; make release",
            "cd /x > out",
            "cd /x | tee out",
            "pushd /x",
            "git status",
            // A `cd` named in a message is not a `cd` being run.
            "git commit -m 'fix: deny a lone cd'",
        ] {
            assert!(!denied(cmd), "{cmd}");
        }
    }
}
