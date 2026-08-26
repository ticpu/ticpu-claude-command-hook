//! One notion of a segment an allow can carry, and the allow built from it.
//!
//! An allow ends the permission decision for the *whole* call, so a chain is
//! covered only when every segment in it is one that grants nothing on its own.
//! Each check that allows a chain used to keep its own list of those, and the
//! lists disagreed: a search beside a `git status` was refused by both, each for
//! the segment the other vouches for.

use crate::checks::git_bypass;
use crate::checks::location::dirs;
use crate::checks::shell;
use crate::input::HookInput;
use crate::output::HookOutput;

const ALLOW_SAFE: &str =
    "a bare `cd`, read-only git, or `git add` on explicit paths (auto-allowed by the hook)";

/// A segment that adds no reach to whatever else the chain is allowed for: it
/// moves the shell, names a variable, prints, or reads. `here` is the directory
/// the segment runs in — `git add` is judged on the paths it names from there.
pub fn is_harmless_segment(segment: &str, here: &str) -> bool {
    // Restated here, not left to the gate in `dispatch`: a substitution runs
    // before the program this classified, so `echo "$(rm -rf /x)"` is neither a
    // print nor harmless.
    !shell::has_substitution(segment)
        && (shell::is_bare_cd(segment)
            || shell::is_bare_assignment(segment)
            || shell::is_read_only_util(segment)
            || git_bypass::is_read_only_segment(segment)
            || git_bypass::is_explicit_add(segment, here))
}

/// Auto-allows a chain of those, so the `cd` Claude Code warns about — hooks from
/// the target directory — has nothing to warn about: none of these runs one.
///
/// A redirect anywhere forfeits it, `2>file` included: it truncates what it names
/// however read-only the command in front of it is.
pub fn allow_chain(input: &HookInput) -> Option<HookOutput> {
    let segments = shell::chain_segments(input.command())?;
    let here = dirs(&segments, &input.cwd);
    let mut works = false;
    for (segment, here) in segments
        .iter()
        .zip(&here)
    {
        if shell::redirects_anything(segment) || !is_harmless_segment(segment, here) {
            return None;
        }
        // A pure reader earns no allow of its own: `ls` and `wc` are already
        // allowlisted, and a bare one reaching here would widen this into the
        // general utility allow it has never been measured as. What is left is
        // the two shapes the allowlist cannot express — a `cd`, and the git
        // commands whose prompt only warns about the hooks they do not run.
        works |= shell::is_bare_cd(segment)
            || git_bypass::is_read_only_segment(segment)
            || git_bypass::is_explicit_add(segment, here);
    }
    works.then(|| HookOutput::allow("PreToolUse", ALLOW_SAFE))
}
