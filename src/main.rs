mod config;
mod import;
mod login;
mod shell;
mod upload;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::Path;
use std::time::Instant;

const VERSION: &str = if cfg!(debug_assertions) {
    "dev"
} else {
    env!("CARGO_PKG_VERSION")
};

#[derive(Parser)]
#[command(name = "giggles", about = "The tool for Giggles librarians", version = VERSION)]
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

    /// Upload one or more GIFs, videos, or tweet URLs
    Upload {
        /// File paths or URLs to upload
        #[arg(required = true)]
        inputs: Vec<String>,

        /// Comma-separated tags (required)
        #[arg(long, value_delimiter = ',', required = true)]
        tags: Vec<String>,

        /// Number of concurrent uploads in batch mode (default: 5)
        #[arg(long, short = 'c', default_value = "5")]
        concurrency: usize,
    },

    /// Import one or more GIFs, videos, or tweet URLs into staging (no tags required)
    Import {
        /// File paths or URLs to import
        #[arg(required = true)]
        inputs: Vec<String>,

        /// Number of concurrent imports in batch mode (default: 5)
        #[arg(long, short = 'c', default_value = "5")]
        concurrency: usize,
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

        Commands::Import {
            inputs,
            concurrency,
        } => {
            let cfg = config::load_config()?;
            if inputs.len() == 1 {
                let input = &inputs[0];
                let started = Instant::now();
                match import::import_one(input, &cfg).await {
                    Ok(_) => {
                        let elapsed = started.elapsed().as_secs_f64();
                        shell::status("Imported", format!("{input} in {elapsed:.2}s"));
                    }
                    Err(e) => {
                        eprintln!("{}", shell::format_error("Failed", e));
                        std::process::exit(1);
                    }
                }
            } else {
                import::import_batch(inputs, cfg, concurrency).await;
            }
        }

        Commands::Upload {
            inputs,
            tags,
            concurrency,
        } => {
            let tags: Vec<String> = tags
                .into_iter()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();

            if tags.is_empty() {
                anyhow::bail!("At least one tag is required");
            }

            let cfg = config::load_config()?;

            if inputs.len() == 1 {
                let input = &inputs[0];
                let name = Path::new(input)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(input)
                    .to_string();
                let started = Instant::now();
                upload::upload_gif(input, &tags, &cfg).await?;
                let elapsed = started.elapsed().as_secs_f64();
                shell::status("Uploaded", format!("`{name}` in {elapsed:.2}s"));
            } else {
                upload::upload_batch(inputs, tags, cfg, concurrency).await;
            }
        }
    }

    Ok(())
}
