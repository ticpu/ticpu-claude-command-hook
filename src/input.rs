use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Default, Deserialize)]
pub struct HookInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub hook_event_name: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub cwd: String,
    /// Kept raw: a check that rewrites the call has to hand back every field the
    /// tool was given, not just the ones this binary knows about.
    #[serde(default)]
    pub tool_input: Value,
}

impl HookInput {
    pub fn command(&self) -> &str {
        self.tool_input_str("command")
    }

    pub fn file_path(&self) -> &str {
        self.tool_input_str("file_path")
    }

    fn tool_input_str(&self, key: &str) -> &str {
        self.tool_input
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}
