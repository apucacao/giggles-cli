use crate::config::Config;
use crate::shell;
use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{Body, Client};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::time::Duration;

const VERCEL_BLOB_API_URL: &str = "https://vercel.com/api/blob";
const BLOB_API_VERSION: &str = "12";
const TWEET_HOSTS: &[&str] = &["twitter.com", "x.com", "fxtwitter.com", "vxtwitter.com"];

// ── Input types ───────────────────────────────────────────────────────────────

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

#[derive(Deserialize)]
struct ImportTweetResponse {
    #[serde(rename = "videoUrl")]
    video_url: String,
}

async fn resolve_input(input: &str, config: &Config, client: &Client) -> Result<ResolvedInput> {
    if is_tweet_url(input) {
        shell::status("Resolving", "tweet media");
        let res = client
            .post(format!("{}/api/gifs/import-tweet", config.server))
            .bearer_auth(&config.api_key)
            .json(&json!({ "tweetUrl": input }))
            .send()
            .await
            .context("Failed to call import-tweet")?;
        if !res.status().is_success() {
            let status = res.status();
            let body: serde_json::Value = res.json().await.unwrap_or_default();
            anyhow::bail!(
                "Server error {}: {}",
                status,
                body["error"].as_str().unwrap_or("unknown")
            );
        }
        let tweet: ImportTweetResponse = res.json().await?;

        let video_url = tweet.video_url;
        shell::status("Downloading", &video_url);
        let res = client
            .get(&video_url)
            .send()
            .await
            .context("Failed to download tweet video")?;
        if !res.status().is_success() {
            anyhow::bail!("Failed to download video: {}", res.status());
        }
        let bytes = res.bytes().await?.to_vec();

        return Ok(ResolvedInput {
            bytes,
            filename: "original.mp4".into(),
            content_type: "video/mp4".into(),
            source: Some(input.to_string()),
        });
    }

    if is_url(input) {
        shell::status("Downloading", input);
        let res = client
            .get(input)
            .send()
            .await
            .context("Failed to download URL")?;
        if !res.status().is_success() {
            anyhow::bail!("Failed to download: {}", res.status());
        }
        let content_type = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let ext = ext_from_content_type(&content_type);
        let bytes = res.bytes().await?.to_vec();

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
        anyhow::bail!("File not found: {input}");
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
        .with_context(|| format!("Failed to read file: {input}"))?;

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
) -> Result<()> {
    let size = bytes.len() as u64;

    let pb = ProgressBar::new(size);
    pb.set_style(
        ProgressStyle::with_template(
            "    Uploading [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, eta {eta})",
        )
        .unwrap()
        .progress_chars("█▉▊▋▌▍▎▏  "),
    );

    let pb_clone = pb.clone();
    let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = bytes
        .chunks(64 * 1024)
        .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
        .collect();

    let stream = futures_util::stream::iter(chunks.into_iter().map(move |chunk| {
        let chunk = chunk.unwrap();
        pb_clone.inc(chunk.len() as u64);
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

    pb.finish_and_clear();

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        anyhow::bail!("Blob upload failed {status}: {body}");
    }

    Ok(())
}

// ── Public entry point ────────────────────────────────────────────────────────

pub async fn upload_gif(input: &str, tags: &[String], config: &Config) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .context("Failed to create HTTP client")?;

    let resolved = resolve_input(input, config, &client).await?;
    let size = resolved.bytes.len();

    shell::status("Preparing", "upload");
    let short_id = prepare_upload(tags, resolved.source.as_deref(), config, &client).await?;

    let pathname = format!("{short_id}/{}", resolved.filename);
    let client_payload = serde_json::to_string(&json!({ "shortId": short_id, "size": size }))?;

    shell::status("Authorizing", "");
    let client_token = get_client_token(&pathname, &client_payload, config, &client).await?;

    upload_to_blob(
        &pathname,
        &resolved.content_type,
        resolved.bytes,
        &client_token,
        &client,
    )
    .await?;

    Ok(short_id)
}
