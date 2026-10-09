//! The user's standing choices, in `$XDG_CONFIG_HOME/ticpu-claude-command-hook/config.yaml`.
//! Markers under the runtime directory are per boot; what is kept here outlives one.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    /// Repos, by directory name, where the comment cap does not fire.
    #[serde(default)]
    pub comment_cap_ignore: Vec<String>,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(
        base.join(env!("CARGO_PKG_NAME"))
            .join("config.yaml"),
    )
}

/// A missing file is the defaults; one that cannot be read or parsed is an error
/// the caller reports.
pub fn load() -> Result<Config> {
    let Some(path) = path() else {
        return Ok(Config::default());
    };
    match fs::read_to_string(&path) {
        Ok(text) => {
            serde_yaml_ng::from_str(&text).with_context(|| format!("parsing {}", path.display()))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The `comment-ignore <repo-name>` verb.
pub fn ignore_comment_cap(repo: &str) -> Result<()> {
    let path = path().context("neither XDG_CONFIG_HOME nor HOME is set")?;
    let mut config = load()?;
    if config
        .comment_cap_ignore
        .iter()
        .any(|name| name == repo)
    {
        println!("{repo} is already ignored in {}", path.display());
        return Ok(());
    }
    config
        .comment_cap_ignore
        .push(repo.to_string());
    let dir = path
        .parent()
        .context("config path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let text = serde_yaml_ng::to_string(&config).context("serializing the config")?;
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    println!("comment cap off in {repo}: {}", path.display());
    Ok(())
}
