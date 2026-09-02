//! `install` / `uninstall` — writes or removes this binary's hook entries in Claude
//! Code's settings.json.
//!
//! Merging is by hook *command*, not by matcher: an entry naming this binary under any
//! path is this binary's, so a moved or renamed checkout re-points instead of leaving a
//! second entry running a stale build. `uninstall` reads the same name the same way, so
//! it takes out the entry a checkout that has since moved left behind.

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
    let binary = binary_path()?;
    let settings = settings_path()?;
    let (before, mut root) = load(&settings)?;

    let replaced = merge(&mut root, &binary.to_string_lossy())?;

    let mut text = serde_json::to_string_pretty(&root).context("serializing settings")?;
    text.push('\n');
    if text == before {
        println!("already installed: {}", settings.display());
        announce_rules(&binary);
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
    announce_rules(&binary);
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let binary = binary_path()?;
    let settings = settings_path()?;
    let (_, mut root) = load(&settings)?;

    let removed = strip(&mut root, &binary.to_string_lossy())?;
    if removed.is_empty() {
        println!("not installed: {}", settings.display());
        return Ok(());
    }

    let mut text = serde_json::to_string_pretty(&root).context("serializing settings")?;
    text.push('\n');
    write_atomically(&settings, &text)?;

    for (event, gone) in removed {
        println!("removed {event} entry: {gone}");
    }
    println!("wrote {}", settings.display());
    println!("Run /hooks or restart to drop it from a session started before now.");
    Ok(())
}

fn binary_path() -> Result<PathBuf> {
    std::env::current_exe()
        .context("resolving this binary's path")?
        .canonicalize()
        .context("canonicalizing this binary's path")
}

/// An absent settings.json is an empty config, not a failure: `install` writes it and
/// `uninstall` then has nothing to take out.
fn load(settings: &Path) -> Result<(String, Value)> {
    let before = match fs::read_to_string(settings) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(e) => return Err(e).context(format!("reading {}", settings.display())),
    };
    let root =
        serde_json::from_str(&before).with_context(|| format!("parsing {}", settings.display()))?;
    Ok((before, root))
}

/// What is auto-allowed is no use to a caller that cannot see it, and the file is
/// generated: `CLAUDE.md` imports it rather than restating it.
fn announce_rules(binary: &Path) {
    match crate::rules::doc_path(binary) {
        Some(path) => println!(
            "Import the allowed shapes from CLAUDE.md: @{}",
            path.display()
        ),
        None => println!("`{} rules` prints the allowed shapes.", binary.display()),
    }
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
    let name = binary_name(binary)?;
    let hooks = hooks_object(root)?;

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

/// Takes this binary out of every event it is named under, not only the ones `ENTRIES`
/// currently lists: an install from an older build may sit under an event since dropped.
/// Returns each command removed with the event it was under — the same path appears once
/// per event, which reads as a repeat unless the event is named beside it.
fn strip(root: &mut Value, binary: &str) -> Result<Vec<(String, String)>> {
    let name = binary_name(binary)?;
    let hooks = hooks_object(root)?;

    let mut removed = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        let groups = groups
            .as_array_mut()
            .with_context(|| format!("settings.json `hooks.{event}` is not an array"))?;
        let mut here = Vec::new();
        for group in groups.iter_mut() {
            prune(group, &name, &mut here);
        }
        groups.retain(|group| !runs_nothing(group));
        removed.extend(
            here.into_iter()
                .map(|command| (event.clone(), command)),
        );
    }
    // An event left with no group at all is our own leftover, so it goes with the entry.
    hooks.retain(|_, groups| {
        !groups
            .as_array()
            .is_some_and(Vec::is_empty)
    });
    Ok(removed)
}

fn binary_name(binary: &str) -> Result<String> {
    Ok(Path::new(binary)
        .file_name()
        .context("this binary has no file name")?
        .to_string_lossy()
        .into_owned())
}

fn hooks_object(root: &mut Value) -> Result<&mut serde_json::Map<String, Value>> {
    root.as_object_mut()
        .context("settings.json is not a JSON object")?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("settings.json `hooks` is not a JSON object")
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

    #[test]
    fn uninstall_leaves_other_hooks_and_settings() {
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
        let removed = strip(&mut root, "/new/ticpu-claude-command-hook").unwrap();

        assert_eq!(
            removed,
            [(
                "PreToolUse".to_string(),
                "/old/ticpu-claude-command-hook".to_string()
            )]
        );
        assert_eq!(root["model"], "opus");
        assert_eq!(commands(&root, "PreToolUse"), ["/usr/local/bin/audit-log"]);
        assert_eq!(commands(&root, "Stop"), ["/usr/local/bin/chime"]);
    }

    /// Nothing of ours may survive an install-then-uninstall round trip, whatever the
    /// events `ENTRIES` names today.
    #[test]
    fn uninstall_undoes_install() {
        let mut root = json!({ "model": "opus" });
        merge(&mut root, "/opt/hook/ticpu-claude-command-hook").unwrap();
        let removed = strip(&mut root, "/opt/hook/ticpu-claude-command-hook").unwrap();

        assert_eq!(removed.len(), ENTRIES.len());
        assert_eq!(root, json!({ "model": "opus", "hooks": {} }));
    }

    #[test]
    fn uninstall_removes_nothing_when_not_installed() {
        let mut root = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [{ "type": "command", "command": "/usr/local/bin/audit-log" }],
                }],
            },
        });
        let before = root.clone();
        let removed = strip(&mut root, "/opt/hook/ticpu-claude-command-hook").unwrap();

        assert!(removed.is_empty());
        assert_eq!(root, before);
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
