//! The file as an `Edit` or `Write` would leave it, for the checks that judge a
//! whole file and not the fragment the tool call carries.

use std::fs;
use std::io::ErrorKind;

use serde_json::Value;

use crate::input::HookInput;

/// The text on disk before the call, empty when the file does not exist yet. Any
/// other read failure is logged and reads as empty too.
pub fn before(input: &HookInput) -> String {
    match fs::read_to_string(input.file_path()) {
        Ok(text) => text,
        Err(e) => {
            if e.kind() != ErrorKind::NotFound {
                eprintln!("edited: reading {}: {e}", input.file_path());
            }
            String::new()
        }
    }
}

/// The text the call leaves behind, given what `before` returned.
pub fn after(input: &HookInput, before: &str) -> String {
    if input.tool_name == "Write" {
        return input
            .content()
            .to_string();
    }
    if before.is_empty() {
        return input
            .new_string()
            .to_string();
    }
    let everywhere = input
        .tool_input
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match everywhere {
        true => before.replace(input.old_string(), input.new_string()),
        false => before.replacen(input.old_string(), input.new_string(), 1),
    }
}
