use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HookOutput {
    #[serde(rename = "systemMessage", skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    pub hook_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

impl HookOutput {
    /// PreToolUse "deny" decision; `reason` is shown to the model.
    pub fn deny(event: &str, reason: &str) -> Self {
        HookOutput {
            system_message: None,
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: event.to_string(),
                permission_decision: Some("deny".to_string()),
                permission_decision_reason: Some(reason.to_string()),
                additional_context: None,
            }),
        }
    }

    /// PostToolUse advisory: a user-facing line plus context injected to the model.
    pub fn context(system: &str, context: &str) -> Self {
        HookOutput {
            system_message: Some(system.to_string()),
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: "PostToolUse".to_string(),
                permission_decision: None,
                permission_decision_reason: None,
                additional_context: Some(context.to_string()),
            }),
        }
    }
}
