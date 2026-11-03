//! Tiny sync HTTP helpers (feature `http`): GitHub API reads and the embeddings POST.

use anyhow::{Context, Result};
use serde_json::Value;

pub fn get_json(url: &str, token: Option<&str>) -> Result<Value> {
    let mut req = ureq::get(url).header("Accept", "application/vnd.github+json").header("User-Agent", "wordcloud");
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    req.call().with_context(|| format!("GET {url}"))?.body_mut().read_json().context("parse JSON")
}

pub fn post_json(url: &str, body: &Value, token: Option<&str>) -> Result<Value> {
    let mut req = ureq::post(url).header("User-Agent", "wordcloud");
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    req.send_json(body).with_context(|| format!("POST {url}"))?.body_mut().read_json().context("parse JSON")
}
