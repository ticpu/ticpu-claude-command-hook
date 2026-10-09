use std::io::Read;

mod checks;
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
        Some(other) => {
            eprintln!(
                "hook: unknown argument {other:?}; the hook JSON is read from stdin, and the verbs are `install`, `uninstall` and `rules`"
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
