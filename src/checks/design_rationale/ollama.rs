//! The one way this check talks to a model. Both reviews share it, so a model,
//! an endpoint or a context length set for one is set for the other.

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

pub(super) fn ask(prompt: &str) -> Result<String> {
    let model = var(MODEL_ENV, DEFAULT_MODEL);
    let url = var(URL_ENV, DEFAULT_URL);
    let body = json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "think": false,
        "options": { "temperature": 0, "num_ctx": NUM_CTX },
    });
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
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
