use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub server: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("giggles")
        .join("config.json")
}

pub fn load_config() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        anyhow::bail!("Not logged in. Run: giggles login --key <api-key> [--server <url>]");
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config: {}", path.display()))?;
    serde_json::from_str(&content).context("Failed to parse config")
}

pub fn save_config(config: &Config) -> Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context("Failed to create config directory")?;
    }
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&path, content)
        .with_context(|| format!("Failed to write config: {}", path.display()))?;
    Ok(())
}
