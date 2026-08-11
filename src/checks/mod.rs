mod broad_find;
mod design_rationale;
mod git_bypass;
mod glab_skill;
mod grep_fold;
mod remote_session;
mod search_flags;
mod search_stderr;
mod shell;

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
            glab_skill::check(input)
                .or_else(|| git_bypass::check(input))
                .or_else(|| broad_find::check(cmd))
                .or_else(|| remote_session::check(cmd))
                .or_else(|| search_stderr::check(cmd))
                .or_else(|| search_flags::check(cmd))
                // Last, in order: an allow ends the chain, so every objection gets
                // first say — and a `git grep` still reaches the fold.
                .or_else(|| grep_fold::check(input))
                .or_else(|| git_bypass::allow_safe(input))
        }
        // MultiEdit is vestigial — Claude Code no longer emits it — so it is not matched.
        "PreToolUse" if input.tool_name == "Edit" || input.tool_name == "Write" => {
            design_rationale::pre_tool_use(input)
        }
        "PostToolUse" => design_rationale::post_tool_use(input.file_path()),
        _ => None,
    }
}
