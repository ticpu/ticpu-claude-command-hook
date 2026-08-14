//! `install` — writes this binary's hook entries into Claude Code's settings.json.
//!
//! Merging is by hook *command*, not by matcher: an entry naming this binary under any
//! path is this binary's, so a moved or renamed checkout re-points instead of leaving a
//! second entry running a stale build.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

/// The events this binary answers, with the tools `checks::dispatch` routes for each.
/// A matcher wider than dispatch spawns a process per unrelated tool call.
const ENTRIES: &[(&str, &str)] = &[
    ("PreToolUse", "Bash|Edit|Write|Read|Grep"),
    ("PostToolUse", "Edit|Write"),
];

pub fn run() -> Result<()> {
    let binary = std::env::current_exe()
        .context("resolving this binary's path")?
        .canonicalize()
        .context("canonicalizing this binary's path")?;
    let settings = settings_path()?;

    let before = match fs::read_to_string(&settings) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(e) => return Err(e).context(format!("reading {}", settings.display())),
    };
    let mut root: Value =
        serde_json::from_str(&before).with_context(|| format!("parsing {}", settings.display()))?;

    let replaced = merge(&mut root, &binary.to_string_lossy())?;

    let mut text = serde_json::to_string_pretty(&root).context("serializing settings")?;
    text.push('\n');
    if text == before {
        println!("already installed: {}", settings.display());
        return Ok(());
    }
    write_atomically(&settings, &text)?;

    for (event, matcher) in ENTRIES {
        println!("{event} {matcher} -> {}", binary.display());
    }
    for stale in replaced {
        println!("replaced stale entry: {stale}");
    }
    println!("wrote {}", settings.display());
    println!("Run /hooks or restart to load it in a session started before now.");
    Ok(())
}

fn settings_path() -> Result<PathBuf> {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is unset")?).join(".claude"),
    };
    Ok(base.join("settings.json"))
}

/// Adds this binary's entry to each event it answers, dropping any entry that already
/// runs a binary of this name. Returns the commands dropped. Every other hook — and
/// every other setting — is left as it stands, so this is safe over a live config.
fn merge(root: &mut Value, binary: &str) -> Result<Vec<String>> {
    let name = Path::new(binary)
        .file_name()
        .context("this binary has no file name")?
        .to_string_lossy()
        .into_owned();

    let hooks = root
        .as_object_mut()
        .context("settings.json is not a JSON object")?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("settings.json `hooks` is not a JSON object")?;

    let mut replaced = Vec::new();
    for (event, matcher) in ENTRIES {
        let groups = hooks
            .entry(*event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .with_context(|| format!("settings.json `hooks.{event}` is not an array"))?;

        for group in groups.iter_mut() {
            prune(group, &name, &mut replaced);
        }
        // A group whose only hook was ours would otherwise sit there matching nothing.
        groups.retain(|group| !runs_nothing(group));
        groups.push(entry(matcher, binary));
    }
    Ok(replaced)
}

fn prune(group: &mut Value, name: &str, replaced: &mut Vec<String>) {
    let Some(hooks) = group
        .get_mut("hooks")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    hooks.retain(|hook| {
        let Some(command) = hook
            .get("command")
            .and_then(Value::as_str)
        else {
            return true;
        };
        let ours = Path::new(command)
            .file_name()
            .is_some_and(|f| f == name);
        if ours {
            replaced.push(command.to_string());
        }
        !ours
    });
}

fn runs_nothing(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

fn entry(matcher: &str, binary: &str) -> Value {
    json!({
        "matcher": matcher,
        "hooks": [{ "type": "command", "command": binary }],
    })
}

/// Same directory, then rename: a settings.json truncated by a failed write costs every
/// setting in it, and Claude Code re-reads the file on change.
fn write_atomically(path: &Path, text: &str) -> Result<()> {
    let dir = path
        .parent()
        .context("settings path has no parent directory")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let temp = path.with_extension("json.new");
    fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    fs::rename(&temp, path)
        .with_context(|| format!("renaming {} to {}", temp.display(), path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(root: &Value, event: &str) -> Vec<String> {
        root["hooks"][event]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| {
                group["hooks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|hook| {
                        hook["command"]
                            .as_str()
                            .unwrap()
                            .to_string()
                    })
            })
            .collect()
    }

    #[test]
    fn writes_both_events_into_an_empty_config() {
        let mut root = json!({});
        let replaced = merge(&mut root, "/opt/hook/ticpu-claude-command-hook").unwrap();

        assert!(replaced.is_empty());
        for (event, matcher) in ENTRIES {
            assert_eq!(root["hooks"][event][0]["matcher"], *matcher);
            assert_eq!(
                commands(&root, event),
                ["/opt/hook/ticpu-claude-command-hook"]
            );
        }
    }

    /// The reinstall case: the same binary from a checkout that has since moved.
    #[test]
    fn a_stale_path_is_replaced_not_duplicated() {
        let mut root = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{ "type": "command", "command": "/old/ticpu-claude-command-hook" }],
                }],
            },
        });
        let replaced = merge(&mut root, "/new/ticpu-claude-command-hook").unwrap();

        assert_eq!(replaced, ["/old/ticpu-claude-command-hook"]);
        assert_eq!(
            commands(&root, "PreToolUse"),
            ["/new/ticpu-claude-command-hook"]
        );
    }

    #[test]
    fn other_hooks_and_settings_survive() {
        let mut root = json!({
            "model": "opus",
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [
                        { "type": "command", "command": "/usr/local/bin/audit-log" },
                        { "type": "command", "command": "/old/ticpu-claude-command-hook" },
                    ],
                }],
                "Stop": [{ "hooks": [{ "type": "command", "command": "/usr/local/bin/chime" }] }],
            },
        });
        merge(&mut root, "/new/ticpu-claude-command-hook").unwrap();

        assert_eq!(root["model"], "opus");
        assert_eq!(
            commands(&root, "PreToolUse"),
            ["/usr/local/bin/audit-log", "/new/ticpu-claude-command-hook"]
        );
        assert_eq!(commands(&root, "Stop"), ["/usr/local/bin/chime"]);
    }

    /// Twice in a row has to leave what once was there — the shape `run` compares to
    /// decide it has nothing to write.
    #[test]
    fn a_second_install_changes_nothing() {
        let mut once = json!({});
        merge(&mut once, "/opt/hook/ticpu-claude-command-hook").unwrap();
        let mut twice = once.clone();
        merge(&mut twice, "/opt/hook/ticpu-claude-command-hook").unwrap();

        assert_eq!(once, twice);
    }
}
