//! The hook binary and its maintenance verbs. On the hook path it reads the hook
//! JSON on stdin and prints the first objection `checks::dispatch` returns, or
//! nothing.
//!
//! It fails open. Bad stdin, a serialize error or a panic exits non-zero with no
//! decision, which Claude Code treats as a non-blocking error, sending the call
//! through its normal permission rules. This binary fronts every tool call in
//! every session, so a refusal emitted by accident takes the whole toolset down
//! until settings.json is edited, while failing open costs only the verdict the
//! failing check owed. Exit 2 blocks, so the hook path never uses it, and
//! `install` must not set `onFailure: "block"`, which would fail closed.

use std::io::Read;

mod checks;
mod config;
mod input;
mod install;
mod output;
mod rules;

use input::HookInput;

fn main() {
    match std::env::args()
        .nth(1)
        .as_deref()
    {
        None => hook(),
        Some("install") => {
            if let Err(e) = install::run() {
                eprintln!("install: {e:#}");
                std::process::exit(1);
            }
        }
        Some("uninstall") => {
            if let Err(e) = install::uninstall() {
                eprintln!("uninstall: {e:#}");
                std::process::exit(1);
            }
        }
        Some("rules") => rules::print(),
        Some("comment-ignore") => {
            let Some(repo) = std::env::args().nth(2) else {
                eprintln!("comment-ignore: name the repo directory to turn the comment cap off in");
                std::process::exit(2);
            };
            if let Err(e) = config::ignore_comment_cap(&repo) {
                eprintln!("comment-ignore: {e:#}");
                std::process::exit(1);
            }
        }
        Some(other) => {
            eprintln!(
                "hook: unknown argument {other:?}; the hook JSON is read from stdin, and the verbs are `install`, `uninstall`, `rules` and `comment-ignore <repo-name>`"
            );
            std::process::exit(2);
        }
    }
}

fn hook() {
    let mut buf = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
        eprintln!("hook: failed reading stdin: {e}");
        std::process::exit(1);
    }

    let input: HookInput = match serde_json::from_str(&buf) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("hook: invalid hook JSON on stdin: {e}");
            std::process::exit(1);
        }
    };

    // No match means "allow" — emit nothing, exit 0.
    let Some(out) = checks::dispatch(&input) else {
        return;
    };

    match serde_json::to_string(&out) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("hook: failed serializing output: {e}");
            std::process::exit(1);
        }
    }
}
