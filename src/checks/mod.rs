mod blind_edit;
mod broad_walk;
mod cargo_tools;
mod cat_read;
mod design_rationale;
mod git_bypass;
mod glab_read_only;
mod glab_skill;
mod grep_fold;
mod literal_assignment;
/// Path resolution every check needs, not git's alone.
mod location;
mod lone_echo;
mod marker;
mod remote_session;
mod search_flags;
mod search_stderr;
mod secret_paths;
mod shell;
mod systemd_read;
mod vouch;

use crate::input::HookInput;
use crate::output::HookOutput;

/// Route a hook event to the applicable checks. The first check to object wins;
/// `None` means every check allowed the action.
pub fn dispatch(input: &HookInput) -> Option<HookOutput> {
    match input
        .hook_event_name
        .as_str()
    {
        "PreToolUse" if input.tool_name == "Bash" => {
            let cmd = input.command();
            // First: the checks below can allow a command outright, and one that
            // prints a credential must never reach an allow.
            secret_paths::waiver_requested(cmd)
                .or_else(|| secret_paths::check(input))
                .or_else(|| glab_skill::check(input))
                .or_else(|| design_rationale::bypass::requested(cmd))
                .or_else(|| design_rationale::disabled::requested(cmd))
                .or_else(|| design_rationale::shell_write::waiver_requested(cmd))
                .or_else(|| design_rationale::shell_write::check(cmd))
                .or_else(|| blind_edit::waiver_requested(cmd))
                .or_else(|| blind_edit::check(cmd))
                .or_else(|| git_bypass::check(input))
                .or_else(|| broad_walk::check(cmd))
                .or_else(|| literal_assignment::check(cmd))
                .or_else(|| remote_session::check(cmd))
                .or_else(|| search_stderr::check(cmd))
                .or_else(|| search_flags::check(cmd))
                // After every other objection: each of them says something this
                // cannot, and `secret_paths` deciding first is what keeps the
                // waiver here away from a credential.
                .or_else(|| cat_read::check(cmd))
                // Last, in order: an allow ends the chain, so every objection gets
                // first say — and a `git grep` still reaches the fold.
                .or_else(|| allows(input))
        }
        // Both hand file contents back as a tool result. Edit and Write do not, so
        // they are not asked here.
        "PreToolUse" if input.tool_name == "Read" || input.tool_name == "Grep" => {
            secret_paths::tool(input)
        }
        // MultiEdit is vestigial — Claude Code no longer emits it — so it is not matched.
        "PreToolUse" if input.tool_name == "Edit" || input.tool_name == "Write" => {
            design_rationale::pre_tool_use(input)
        }
        // The gate already reviewed it; this only says so, where the writer can read
        // it. A permission prompt's reason cannot: it is addressed to the reader.
        "PostToolUse" if input.tool_name == "Edit" || input.tool_name == "Write" => {
            design_rationale::post_tool_use(input)
        }
        _ => None,
    }
}

/// The checks that can end the permission decision outright. A command substitution
/// bars all of them: it runs before the program the check classified, so a `git log`
/// or a `cargo test` vouches for nothing once `$( )` is in its arguments. The
/// substitution is refused as a whole — telling an inert `$(pwd)` from a `$(curl | sh)`
/// is the classification the check itself was written to avoid needing.
fn allows(input: &HookInput) -> Option<HookOutput> {
    let cmd = input.command();
    // Ahead of the gate, and the only thing that goes there: a quote-delimited
    // heredoc body is literal, so a `$( )` or an apostrophe in a commit message is
    // text. This check applies the same gate to the head, which is what runs.
    if let Some(allowed) = vouch::allow_heredoc_commit(input) {
        return Some(allowed);
    }
    if shell::has_substitution(cmd) {
        return None;
    }
    grep_fold::check(input)
        .or_else(|| vouch::allow_chain(input))
        .or_else(|| glab_read_only::allow(cmd))
        .or_else(|| systemd_read::allow(cmd))
        .or_else(|| cargo_tools::allow(cmd))
        .or_else(|| lone_echo::allow(cmd))
        .or_else(|| cat_read::waiver_allowed(cmd))
}
