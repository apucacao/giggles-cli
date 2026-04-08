use crate::config::{Config, save_config};
use crate::shell;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::time::sleep;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const TIMEOUT: Duration = Duration::from_secs(600); // 10 minutes

#[derive(Deserialize)]
struct RequestResponse {
    #[serde(rename = "deviceCode")]
    device_code: String,
    #[serde(rename = "userCode")]
    user_code: String,
    #[serde(rename = "verificationUrl")]
    verification_url: String,
}

#[derive(Deserialize)]
struct PollResponse {
    status: String,
    #[serde(rename = "apiKey")]
    api_key: Option<String>,
}

pub async fn run(server: &str) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    // 1. Request a device code
    let res = client
        .post(format!("{server}/api/cli/auth/request"))
        .send()
        .await
        .context("Failed to reach server")?;

    if !res.status().is_success() {
        anyhow::bail!("Server error: {}", res.status());
    }

    let request: RequestResponse = res.json().await.context("Invalid response from server")?;

    // 2. Open browser and prompt user
    shell::status("Opening", "browser to authorize");
    if let Err(e) = open::that(&request.verification_url) {
        shell::warn(format!("Could not open browser: {e}"));
        shell::status("Visit", &request.verification_url);
    }

    shell::status(
        "Waiting",
        format!(
            "confirm code {} in your browser",
            request.user_code
        ),
    );

    // 3. Poll until approved, expired, or timed out
    let started = Instant::now();
    loop {
        if started.elapsed() > TIMEOUT {
            anyhow::bail!("Timed out waiting for authorization");
        }

        sleep(POLL_INTERVAL).await;

        let res = client
            .get(format!("{server}/api/cli/auth/poll"))
            .query(&[("deviceCode", &request.device_code)])
            .send()
            .await
            .context("Poll request failed")?;

        if !res.status().is_success() {
            continue; // transient error, keep polling
        }

        let poll: PollResponse = res.json().await.context("Invalid poll response")?;

        match poll.status.as_str() {
            "approved" => {
                let api_key = poll.api_key.context("Server approved but sent no key")?;
                let server = server.trim_end_matches('/').to_string();
                save_config(&Config { server, api_key })?;
                shell::status("Authorized", "you're all set");
                return Ok(());
            }
            "expired" => anyhow::bail!("Code expired. Run `giggles login` again."),
            _ => {} // pending — keep polling
        }
    }
}
