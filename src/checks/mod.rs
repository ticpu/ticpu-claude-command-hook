mod broad_find;
mod design_rationale;
mod git_bypass;
mod glab_skill;
mod grep_fold;

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
                // Last: a rewrite also allows, so every objection gets first say.
                .or_else(|| grep_fold::check(input))
        }
        "PostToolUse" => design_rationale::check(input.file_path()),
        _ => None,
    }
}
