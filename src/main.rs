mod config;
mod login;
mod shell;
mod upload;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::Path;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "giggles", about = "Upload GIFs, videos, and tweets to Giggles")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Authenticate with Giggles (opens browser)
    Login {
        /// Server URL
        #[arg(long, default_value = "https://gggl.es")]
        server: String,
    },

    /// Upload a GIF, video, or tweet URL
    Upload {
        /// File path or URL to upload
        input: String,

        /// Comma-separated tags (required)
        #[arg(long, value_delimiter = ',', required = true)]
        tags: Vec<String>,
    },

    /// Show the authenticated user
    Whoami,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { server } => {
            login::run(&server).await?;
        }

        Commands::Whoami => {
            let cfg = config::load_config()?;
            let client = reqwest::Client::new();
            let res = client
                .get(format!("{}/api/cli/auth/whoami", cfg.server))
                .bearer_auth(&cfg.api_key)
                .send()
                .await
                .context("Failed to reach server")?;
            if res.status() == 401 {
                anyhow::bail!("Not authenticated. Run `giggles login`.");
            }
            let body: serde_json::Value = res.json().await.context("Invalid response")?;
            let email = body["email"].as_str().unwrap_or("unknown");
            shell::status("Logged in", email);
        }

        Commands::Upload { input, tags } => {
            let tags: Vec<String> = tags
                .into_iter()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();

            if tags.is_empty() {
                anyhow::bail!("At least one tag is required");
            }

            let cfg = config::load_config()?;
            let name = Path::new(&input)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&input)
                .to_string();
            let started = Instant::now();
            upload::upload_gif(&input, &tags, &cfg).await?;
            let elapsed = started.elapsed().as_secs_f64();
            shell::status("Uploaded", format!("`{name}` in {elapsed:.2}s"));
        }
    }

    Ok(())
}
