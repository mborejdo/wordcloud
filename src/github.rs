//! Public GitHub activity (feature `http`): languages and topics of a user's most recently pushed
//! public repos, as counts. No repo names or descriptions leave this module - and the cloud only
//! uses terms that fold into a curated word anyway.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::Value;

/// `(term, repos)` for the `take` most recently pushed public, non-fork, non-archived repos of
/// `user`. Token from `$GITHUB_TOKEN` if set (only for the rate limit; public data either way).
pub fn activity(user: &str, take: usize) -> Result<Vec<(String, u32)>> {
    let token = std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty());
    let url = format!("https://api.github.com/users/{user}/repos?type=owner&sort=pushed&direction=desc&per_page=100");
    let repos = crate::http::get_json(&url, token.as_deref())?;
    Ok(summarize(repos.as_array().map(Vec::as_slice).unwrap_or_default(), take))
}

/// Count languages and topics over the most recently pushed repos. Pure, for tests and callers
/// that fetch the repo list themselves.
pub fn summarize(repos: &[Value], take: usize) -> Vec<(String, u32)> {
    let mut recent: Vec<&Value> = repos.iter().filter(|r| r["fork"] != true && r["archived"] != true && r["private"] != true).collect();
    recent.sort_by(|a, b| b["pushed_at"].as_str().unwrap_or_default().cmp(a["pushed_at"].as_str().unwrap_or_default()));
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for r in recent.iter().take(take) {
        let lang = r["language"].as_str().map(str::to_owned);
        let topics = r["topics"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned);
        for t in lang.into_iter().chain(topics) {
            *counts.entry(t).or_default() += 1;
        }
    }
    counts.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_recent_public_repos_only() {
        let repos = vec![
            json!({"language": "Rust", "topics": ["kubernetes"], "pushed_at": "2026-11-01T00:00:00Z"}),
            json!({"language": "Rust", "topics": [], "pushed_at": "2026-10-01T00:00:00Z"}),
            json!({"language": "Go", "fork": true, "pushed_at": "2026-11-02T00:00:00Z"}),
            json!({"language": "TypeScript", "private": true, "pushed_at": "2026-11-02T00:00:00Z"}),
            json!({"language": "PHP", "pushed_at": "2020-01-01T00:00:00Z"}),
        ];
        assert_eq!(summarize(&repos, 2), vec![("Rust".into(), 2), ("kubernetes".into(), 1)]);
    }
}
