//! The one way this check talks to a model. Both reviews share it, so a model,
//! an endpoint or a context length set for one is set for the other.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const MODEL_ENV: &str = "CLAUDE_HOOK_JUDGE_MODEL";
const URL_ENV: &str = "CLAUDE_HOOK_JUDGE_URL";
const DEFAULT_MODEL: &str = "gemma4:12b";
const DEFAULT_URL: &str = "http://localhost:11434/api/generate";

/// Stated on every request rather than taken from the server's configuration: a
/// short default truncates the document out of the prompt, and a truncated prompt
/// does not fail — the model answers from whatever survived.
const NUM_CTX: u32 = 32768;

/// Generous against a warm model, and still inside the hook budget when it has to
/// be loaded first.
const TIMEOUT: Duration = Duration::from_secs(45);

/// A cold load from disk outlasts it, and is the one wait worth sitting through.
const LOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The verb `main` runs `load` under.
pub const LOAD_VERB: &str = "load-judge";

pub(super) fn ask(prompt: &str) -> Result<String> {
    post(
        json!({
            "prompt": prompt,
            "stream": false,
            "think": false,
            "options": { "temperature": 0, "num_ctx": NUM_CTX },
        }),
        TIMEOUT,
    )
}

/// Starts `load` in a process of its own, so a cold load outlives this hook: ollama
/// aborts a load whose client disconnects, and the judge's own timeout is that client.
pub(super) fn warm() {
    let spawned = std::env::current_exe().and_then(|exe| {
        Command::new(exe)
            .arg(LOAD_VERB)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
    });
    if let Err(e) = spawned {
        eprintln!("design_rationale: starting the judge model load failed: {e}");
    }
}

/// A request with no prompt loads the model and returns. `num_ctx` matches `ask`'s,
/// or ollama reloads at the judge's first call.
pub fn load() -> Result<()> {
    post(json!({ "options": { "num_ctx": NUM_CTX } }), LOAD_TIMEOUT).map(drop)
}

fn post(mut body: Value, timeout: Duration) -> Result<String> {
    let model = var(MODEL_ENV, DEFAULT_MODEL);
    let url = var(URL_ENV, DEFAULT_URL);
    body["model"] = json!(model);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    let mut response = agent
        .post(&url)
        .send_json(&body)
        .with_context(|| format!("POST {url} ({model})"))?;
    let value: Value = response
        .body_mut()
        .read_json()
        .context("decoding the ollama reply")?;
    match value
        .get("response")
        .and_then(Value::as_str)
    {
        Some(text) => Ok(text.to_string()),
        None => bail!("ollama replied without a `response` field"),
    }
}

fn var(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}
