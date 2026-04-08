use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[cfg(debug_assertions)]
const DEFAULT_SERVER: &str = "http://localhost:3000";

#[cfg(not(debug_assertions))]
const DEFAULT_SERVER: &str = "https://gggl.es";

#[derive(Serialize, Deserialize, Default)]
struct FileConfig {
    #[serde(default)]
    servers: HashMap<String, ServerEntry>,
}

#[derive(Serialize, Deserialize)]
struct ServerEntry {
    api_key: String,
}

pub struct Config {
    pub server: String,
    pub api_key: String,
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        });
    base.join("giggles").join("config.toml")
}

pub fn load_config() -> Result<Config> {
    let server = std::env::var("GIGGLES_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_string());

    let path = config_path();
    if !path.exists() {
        anyhow::bail!("Not logged in. Run `giggles login`.");
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config: {}", path.display()))?;
    let file: FileConfig = toml::from_str(&content).context("Failed to parse config")?;

    let entry = file.servers.get(&server).with_context(|| {
        format!("No credentials for {server}. Run `giggles login --server {server}`.")
    })?;

    Ok(Config {
        server,
        api_key: entry.api_key.clone(),
    })
}

pub fn save_config(server: &str, api_key: &str) -> Result<()> {
    let path = config_path();
    let mut file: FileConfig = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        FileConfig::default()
    };

    file.servers.insert(
        server.to_string(),
        ServerEntry {
            api_key: api_key.to_string(),
        },
    );

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context("Failed to create config directory")?;
    }
    fs::write(&path, toml::to_string_pretty(&file)?)
        .with_context(|| format!("Failed to write config: {}", path.display()))?;
    Ok(())
}
