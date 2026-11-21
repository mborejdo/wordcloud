//! Small blocking HTTP helpers (feature `http`): GitHub API reads and the embeddings POST.

use anyhow::{Context, Result};
use serde_json::Value;

/// GET `url` as JSON, with a bearer token if given.
pub(crate) fn get_json(url: &str, token: Option<&str>) -> Result<Value> {
    let mut request = ureq::get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "wordcloud");
    if let Some(token) = token {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    request
        .call()
        .with_context(|| format!("GET {url}"))?
        .body_mut()
        .read_json()
        .context("parse JSON")
}

/// POST `body` to `url` as JSON and read JSON back, with a bearer token if given.
pub(crate) fn post_json(url: &str, body: &Value, token: Option<&str>) -> Result<Value> {
    let mut request = ureq::post(url).header("User-Agent", "wordcloud");
    if let Some(token) = token {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    request
        .send_json(body)
        .with_context(|| format!("POST {url}"))?
        .body_mut()
        .read_json()
        .context("parse JSON")
}
