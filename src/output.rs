use serde::Serialize;
use serde_json::Value;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<Value>,
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
                updated_input: None,
            }),
        }
    }

    /// PreToolUse "allow" decision: bypasses the normal permission prompt.
    pub fn allow(event: &str, reason: &str) -> Self {
        HookOutput {
            system_message: None,
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: event.to_string(),
                permission_decision: Some("allow".to_string()),
                permission_decision_reason: Some(reason.to_string()),
                additional_context: None,
                updated_input: None,
            }),
        }
    }

    /// PreToolUse rewrite: the tool runs `updated_input` instead of what the
    /// model sent. Claude Code only honours it alongside an "allow" decision,
    /// so a rewritten command also skips the permission prompt.
    pub fn rewrite(event: &str, reason: &str, updated_input: Value) -> Self {
        HookOutput {
            system_message: None,
            hook_specific_output: Some(HookSpecificOutput {
                hook_event_name: event.to_string(),
                permission_decision: Some("allow".to_string()),
                permission_decision_reason: Some(reason.to_string()),
                additional_context: None,
                updated_input: Some(updated_input),
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
                updated_input: None,
            }),
        }
    }
}
