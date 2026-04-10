use crate::config::Config;
use crate::shell;
use anyhow::{Context, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::{Body, Client};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

const VERCEL_BLOB_API_URL: &str = "https://vercel.com/api/blob";
const BLOB_API_VERSION: &str = "12";
const TWEET_HOSTS: &[&str] = &["twitter.com", "x.com", "fxtwitter.com", "vxtwitter.com"];

// ── Input resolution ──────────────────────────────────────────────────────────

struct ResolvedInput {
    bytes: Vec<u8>,
    filename: String,
    content_type: String,
    source: Option<String>,
}

fn is_tweet_url(input: &str) -> bool {
    if let Ok(url) = url::Url::parse(input) {
        if let Some(host) = url.host_str() {
            return TWEET_HOSTS
                .iter()
                .any(|&h| host == h || host.ends_with(&format!(".{h}")));
        }
    }
    false
}

fn is_url(input: &str) -> bool {
    matches!(url::Url::parse(input), Ok(u) if u.scheme() == "http" || u.scheme() == "https")
}

fn ext_from_content_type(ct: &str) -> &str {
    if ct.contains("gif") {
        "gif"
    } else if ct.contains("webm") {
        "webm"
    } else {
        "mp4"
    }
}

enum ResolveError {
    /// Tweet has no usable media (4xx from import-tweet)
    Skip(String),
    /// Network or server error
    Fail(anyhow::Error),
}

#[derive(Deserialize)]
struct ImportTweetResponse {
    #[serde(rename = "videoUrl")]
    video_url: String,
}

async fn resolve_input(
    input: &str,
    config: &Config,
    client: &Client,
    verbose: bool,
) -> Result<ResolvedInput, ResolveError> {
    if is_tweet_url(input) {
        if verbose {
            shell::status("Resolving", "tweet media");
        }
        let res = client
            .post(format!("{}/api/gifs/import-tweet", config.server))
            .bearer_auth(&config.api_key)
            .json(&json!({ "tweetUrl": input }))
            .send()
            .await
            .map_err(|e| ResolveError::Fail(anyhow::anyhow!("Failed to call import-tweet: {e}")))?;

        if res.status().is_client_error() {
            let body: serde_json::Value = res.json().await.unwrap_or_default();
            let msg = body["error"]
                .as_str()
                .unwrap_or("no animated GIF")
                .to_string();
            return Err(ResolveError::Skip(msg));
        }
        if !res.status().is_success() {
            return Err(ResolveError::Fail(anyhow::anyhow!(
                "Server error {}",
                res.status()
            )));
        }

        let tweet: ImportTweetResponse = res.json().await.map_err(|e| {
            ResolveError::Fail(anyhow::anyhow!("Invalid import-tweet response: {e}"))
        })?;

        let video_url = tweet.video_url;
        if verbose {
            shell::status("Downloading", &video_url);
        }
        let res = client.get(&video_url).send().await.map_err(|e| {
            ResolveError::Fail(anyhow::anyhow!("Failed to download tweet video: {e}"))
        })?;
        if !res.status().is_success() {
            return Err(ResolveError::Fail(anyhow::anyhow!(
                "Failed to download video: {}",
                res.status()
            )));
        }
        let bytes = res
            .bytes()
            .await
            .map_err(|e| ResolveError::Fail(anyhow::anyhow!("{e}")))?
            .to_vec();

        return Ok(ResolvedInput {
            bytes,
            filename: "original.mp4".into(),
            content_type: "video/mp4".into(),
            source: Some(input.to_string()),
        });
    }

    if is_url(input) {
        if verbose {
            shell::status("Downloading", input);
        }
        let res = client
            .get(input)
            .send()
            .await
            .map_err(|e| ResolveError::Fail(anyhow::anyhow!("Failed to download URL: {e}")))?;
        if !res.status().is_success() {
            return Err(ResolveError::Fail(anyhow::anyhow!(
                "Failed to download: {}",
                res.status()
            )));
        }
        let content_type = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let ext = ext_from_content_type(&content_type);
        let bytes = res
            .bytes()
            .await
            .map_err(|e| ResolveError::Fail(anyhow::anyhow!("{e}")))?
            .to_vec();

        return Ok(ResolvedInput {
            bytes,
            filename: format!("original.{ext}"),
            content_type,
            source: Some(input.to_string()),
        });
    }

    // Local file
    let path = Path::new(input);
    if !path.exists() {
        return Err(ResolveError::Fail(anyhow::anyhow!(
            "File not found: {input}"
        )));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("gif")
        .to_string();
    let content_type = match ext.as_str() {
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    };
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ResolveError::Fail(anyhow::anyhow!("Failed to read file {input}: {e}")))?;

    Ok(ResolvedInput {
        bytes,
        filename: format!("original.{ext}"),
        content_type: content_type.to_string(),
        source: None,
    })
}

// ── Upload API calls ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PrepareResponse {
    #[serde(rename = "shortId")]
    short_id: String,
}

#[derive(Serialize)]
struct PrepareRequest<'a> {
    tags: &'a [String],
    source: Option<&'a str>,
}

async fn prepare_upload(
    tags: &[String],
    source: Option<&str>,
    config: &Config,
    client: &Client,
) -> Result<String> {
    let res = client
        .post(format!("{}/api/upload/prepare", config.server))
        .bearer_auth(&config.api_key)
        .json(&PrepareRequest { tags, source })
        .send()
        .await
        .context("Failed to call /api/upload/prepare")?;

    if !res.status().is_success() {
        let status = res.status();
        let body: serde_json::Value = res.json().await.unwrap_or_default();
        anyhow::bail!(
            "Prepare failed {}: {}",
            status,
            body["error"].as_str().unwrap_or("unknown")
        );
    }

    let resp: PrepareResponse = res.json().await.context("Invalid prepare response")?;
    Ok(resp.short_id)
}

#[derive(Deserialize)]
struct ClientTokenResponse {
    #[serde(rename = "clientToken")]
    client_token: String,
}

async fn get_client_token(
    pathname: &str,
    client_payload: &str,
    config: &Config,
    client: &Client,
) -> Result<String> {
    let body = json!({
        "type": "blob.generate-client-token",
        "payload": {
            "pathname": pathname,
            "clientPayload": client_payload,
            "multipart": false,
        }
    });

    let res = client
        .post(format!("{}/api/upload", config.server))
        .bearer_auth(&config.api_key)
        .json(&body)
        .send()
        .await
        .context("Failed to get client token")?;

    if !res.status().is_success() {
        let status = res.status();
        let body: serde_json::Value = res.json().await.unwrap_or_default();
        anyhow::bail!(
            "Token generation failed {}: {}",
            status,
            body["error"].as_str().unwrap_or("unknown")
        );
    }

    let resp: ClientTokenResponse = res.json().await.context("Invalid token response")?;
    Ok(resp.client_token)
}

async fn upload_to_blob(
    pathname: &str,
    content_type: &str,
    bytes: Vec<u8>,
    client_token: &str,
    client: &Client,
    verbose: bool,
) -> Result<()> {
    let size = bytes.len() as u64;

    let pb = if verbose {
        let pb = ProgressBar::new(size);
        pb.set_style(
            ProgressStyle::with_template(
                "    Uploading [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})",
            )
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏  "),
        );
        Some(pb)
    } else {
        None
    };

    let pb_clone = pb.clone();
    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = bytes
        .chunks(64 * 1024)
        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
        .collect();

    let stream = futures_util::stream::iter(chunks.into_iter().map(move |chunk| {
        let chunk = chunk.unwrap();
        if let Some(ref pb) = pb_clone {
            pb.inc(chunk.len() as u64);
        }
        Ok::<_, std::io::Error>(chunk)
    }));

    let url = format!("{VERCEL_BLOB_API_URL}/?pathname={pathname}");
    let request_id = uuid::Uuid::new_v4().to_string();

    let res = client
        .put(&url)
        .header("authorization", format!("Bearer {client_token}"))
        .header("x-api-version", BLOB_API_VERSION)
        .header("x-api-blob-request-id", &request_id)
        .header("x-api-blob-request-attempt", "0")
        .header("x-vercel-blob-access", "private")
        .header("x-content-type", content_type)
        .header("x-add-random-suffix", "0")
        .header("x-content-length", size.to_string())
        .body(Body::wrap_stream(stream))
        .send()
        .await
        .context("Failed to upload to Vercel Blob")?;

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        anyhow::bail!("Blob upload failed {status}: {body}");
    }

    Ok(())
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

enum PipelineError {
    Skip(String),
    Fail(String),
}

async fn run_pipeline(
    input: &str,
    tags: &[String],
    config: &Config,
    client: &Client,
    verbose: bool,
) -> Result<String, PipelineError> {
    let resolved = resolve_input(input, config, client, verbose)
        .await
        .map_err(|e| match e {
            ResolveError::Skip(s) => PipelineError::Skip(s),
            ResolveError::Fail(e) => PipelineError::Fail(e.to_string()),
        })?;

    let size = resolved.bytes.len();

    if verbose {
        shell::status("Preparing", "upload");
    }
    let short_id = prepare_upload(tags, resolved.source.as_deref(), config, client)
        .await
        .map_err(|e| PipelineError::Fail(e.to_string()))?;

    let pathname = format!("{short_id}/{}", resolved.filename);
    let client_payload = serde_json::to_string(&json!({ "shortId": short_id, "size": size }))
        .map_err(|e| PipelineError::Fail(e.to_string()))?;

    if verbose {
        shell::status("Authorizing", "");
    }
    let client_token = get_client_token(&pathname, &client_payload, config, client)
        .await
        .map_err(|e| PipelineError::Fail(e.to_string()))?;

    upload_to_blob(
        &pathname,
        &resolved.content_type,
        resolved.bytes,
        &client_token,
        client,
        verbose,
    )
    .await
    .map_err(|e| PipelineError::Fail(e.to_string()))?;

    Ok(short_id)
}

// ── Public entry points ───────────────────────────────────────────────────────

pub async fn upload_gif(input: &str, tags: &[String], config: &Config) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .context("Failed to create HTTP client")?;

    run_pipeline(input, tags, config, &client, true)
        .await
        .map_err(|e| match e {
            PipelineError::Skip(msg) => anyhow::anyhow!("Skipped: {msg}"),
            PipelineError::Fail(msg) => anyhow::anyhow!("{msg}"),
        })
}

enum ItemOutcome {
    Uploaded,
    Skipped(String),
    Failed(String),
}

pub async fn upload_batch(
    inputs: Vec<String>,
    tags: Vec<String>,
    config: Config,
    concurrency: usize,
) {
    let total = inputs.len() as u64;
    let mp = MultiProgress::new();
    let pb = mp.add(ProgressBar::new(total));
    pb.set_style(
        ProgressStyle::with_template("    Progress [{bar:40.cyan/blue}] {pos}/{len}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏  "),
    );

    let sem = Arc::new(Semaphore::new(concurrency));
    let config = Arc::new(config);
    let tags = Arc::new(tags);
    let client = match Client::builder().timeout(Duration::from_secs(300)).build() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            shell::error(format!("Failed to create HTTP client: {e}"));
            return;
        }
    };

    let started = Instant::now();
    let mut handles = vec![];

    for input in inputs {
        let sem = sem.clone();
        let cfg = config.clone();
        let tgs = tags.clone();
        let cli = client.clone();
        let mp_clone = mp.clone();
        let pb_clone = pb.clone();

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let t0 = Instant::now();

            let outcome = match run_pipeline(&input, &tgs, &cfg, &cli, false).await {
                Ok(_) => ItemOutcome::Uploaded,
                Err(PipelineError::Skip(s)) => ItemOutcome::Skipped(s),
                Err(PipelineError::Fail(s)) => ItemOutcome::Failed(s),
            };

            let elapsed = t0.elapsed().as_secs_f64();
            let line = match &outcome {
                ItemOutcome::Uploaded => {
                    shell::format_status("Uploaded", format!("{input} ({elapsed:.1}s)"))
                }
                ItemOutcome::Skipped(reason) => {
                    shell::format_warn("Skipped", format!("{input}: {reason}"))
                }
                ItemOutcome::Failed(err) => {
                    shell::format_error("Failed", format!("{input}: {err}"))
                }
            };
            let _ = mp_clone.println(line);
            pb_clone.inc(1);

            (input, outcome)
        }));
    }

    let mut results = vec![];
    for handle in handles {
        if let Ok(r) = handle.await {
            results.push(r);
        }
    }

    pb.finish_and_clear();

    let uploaded = results
        .iter()
        .filter(|(_, o)| matches!(o, ItemOutcome::Uploaded))
        .count();
    let skipped = results
        .iter()
        .filter(|(_, o)| matches!(o, ItemOutcome::Skipped(_)))
        .count();
    let failed: Vec<_> = results
        .iter()
        .filter(|(_, o)| matches!(o, ItemOutcome::Failed(_)))
        .collect();

    let secs_total = started.elapsed().as_secs();
    let time_str = if secs_total >= 60 {
        format!("{}m {}s", secs_total / 60, secs_total % 60)
    } else {
        format!("{secs_total}s")
    };

    shell::status(
        "Finished",
        format!(
            "in {time_str} — {uploaded} uploaded, {skipped} skipped, {} failed",
            failed.len()
        ),
    );

    for (input, outcome) in &failed {
        if let ItemOutcome::Failed(err) = outcome {
            shell::error(format!("{input}: {err}"));
        }
    }
}
