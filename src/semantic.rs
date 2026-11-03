//! Semantic weighting before layout: related terms boost a head word.
//!
//! Input: the head words from the word list (tiers + `[[word]]`), a raw `terms` list and an
//! optional text corpus (e.g. a CV). Output: the final word list, each with the reasons for its weight.
//!
//! - every raw term is offered to a [`Scorer`]; if it belongs to a head word ("tsc" ->
//!   TypeScript), the head gets `term_boost` (capped at `term_cap`). Terms that match nothing
//!   become new words, and later terms can fold into those - that is the auto clustering.
//! - every word then gets `mention_boost` per mention in the corpus (capped at `mention_cap`), so
//!   what the text actually says counts. Longer phrases win: "React Native" is not also "React".
//!
//! Backends: [`Offline`] (default; normalisation, light stemming, built-in alias table) and
//! [`Embed`] (opt-in; any OpenAI-compatible embeddings endpoint, e.g. a local Ollama). The
//! embed backend sends only the word list, nothing from the corpus.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::source::Word;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// "offline" (default) or "embed".
    pub backend: String,
    pub term_boost: f32,
    pub term_cap: f32,
    pub mention_boost: f32,
    pub mention_cap: u32,
    /// Weight and colour for words that only come from `terms`.
    pub new_weight: f32,
    pub new_color: String,
    /// Embed backend: OpenAI-compatible `/v1/embeddings` URL, model, min cosine similarity.
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub threshold: f32,
    /// `cv cloud --github`: boost per recent repo using a language/topic, and the cap per word.
    pub github_boost: f32,
    pub github_cap: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: "offline".into(),
            term_boost: 0.35,
            term_cap: 1.0,
            mention_boost: 0.15,
            mention_cap: 6,
            new_weight: 3.0,
            new_color: "soft".into(),
            endpoint: None,
            model: None,
            threshold: 0.8,
            github_boost: 0.1,
            github_cap: 0.5,
        }
    }
}

/// Which head a term belongs to, and why.
pub struct Match {
    pub head: usize,
    pub why: String,
}

pub trait Scorer {
    fn assign(&self, heads: &[Word], term: &str) -> Option<Match>;
}

// ---------------------------------------------------------------- normalisation

/// Lowercase, alphanumerics plus `#` / `+` only: "Next.js" -> "nextjs", "OAuth 2.1" -> "oauth21".
pub fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || *c == '#' || *c == '+').flat_map(char::to_lowercase).collect()
}

/// Very light stemming on a normalised key: "nextjs" -> "next", "apis" -> "api".
pub fn stem(k: &str) -> String {
    let k = k.strip_suffix("js").filter(|r| r.len() >= 3).unwrap_or(k);
    let k = if k.len() > 3 && k.ends_with('s') && !k.ends_with("ss") { &k[..k.len() - 1] } else { k };
    k.to_owned()
}

/// Phrase tokens, each normalised and stemmed: "API-Gateway" -> ["api", "gateway"].
fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || matches!(c, '#' | '+' | '.'))).map(|t| stem(&norm(t))).filter(|t| !t.is_empty()).collect()
}

/// Built-in aliases, keyed by the normalised head. Only used when that head is in the list.
const ALIASES: &[(&str, &[&str])] = &[
    ("typescript", &["ts", "tsc", "typedjavascript"]),
    ("javascript", &["js", "ecmascript", "es6", "vanillajs"]),
    ("rust", &["rustlang", "rustc", "cargo"]),
    ("kubernetes", &["k8s", "kube", "kubectl", "aks", "k3s", "helm"]),
    ("gitops", &["argocd", "argo", "flux", "fluxcd"]),
    ("devops", &["cicd", "ci", "cd", "continuousdelivery", "continuousintegration", "pipelines"]),
    ("docker", &["container", "dockerfile", "podman", "compose"]),
    ("postgresql", &["postgres", "psql", "pg"]),
    ("nextjs", &["next", "approuter"]),
    ("react", &["reactjs", "jsx", "hooks"]),
    ("nodejs", &["node", "npm", "pnpm"]),
    ("vue", &["vuejs", "vue3", "nuxt"]),
    ("oauth21", &["oauth", "oauth2", "oauth20"]),
    ("oidc", &["openidconnect", "openid"]),
    ("mcp", &["modelcontextprotocol", "mcpserver"]),
    ("aiagents", &["agent", "agentic", "llmagent", "ai", "llm", "ki"]),
    ("rag", &["retrievalaugmentedgeneration"]),
    ("terraform", &["tf", "opentofu", "hcl"]),
    ("infrastructureascode", &["iac"]),
    ("openapi", &["swagger"]),
    ("rest", &["restful", "restapi"]),
    ("apis", &["apidesign", "http"]),
    ("apigateway", &["gateway"]),
    ("azure", &["microsoftazure", "entra", "entraid"]),
    ("linux", &["unix", "debian", "ubuntu"]),
    ("sql", &["mysql", "sqlite"]),
    ("php", &["symfony", "laravel", "composer", "phpunit"]),
    ("webcomponents", &["customelement", "shadowdom"]),
    ("lit", &["litelement"]),
    ("tailwind", &["tailwindcss"]),
    ("c#", &["csharp", "dotnet", "net"]),
    ("python", &["py"]),
    ("architecture", &["softwarearchitecture", "systemarchitecture"]),
    ("systemdesign", &["systemsdesign"]),
    ("platformengineering", &["platform", "idp", "internaldeveloperplatform"]),
    ("b2bcommerce", &["ecommerce", "b2b", "shop", "onlineshop", "commerce"]),
    ("headlesscommerce", &["headless"]),
    ("mediasystemdesign", &["msd", "mediasystem"]),
    ("reactnative", &["rn", "expo"]),
    ("stibodx", &["cue", "stibodxcue"]),
    ("fullstack", &["fullstackdeveloper", "fullstackengineer"]),
    ("automatedtests", &["testing", "test", "tdd", "e2e", "unittest", "playwright", "jest"]),
    ("documentation", &["doc", "dokumentation"]),
    ("validation", &["computersystemvalidation", "csv"]),
    ("gxp", &["gmp", "gdp", "gcp", "glp"]),
    ("erp", &["sap"]),
    ("pim", &["pimcore", "productinformation"]),
    ("search", &["algolia", "meilisearch", "solr", "elasticsearch"]),
    ("payments", &["stripe"]),
    ("developerportals", &["backstage", "devportal"]),
    ("tooling", &["tool", "cli"]),
    ("redis", &["dragonfly", "valkey"]),
    ("objectstorage", &["s3", "blob", "blobstorage"]),
    ("flash", &["adobeflash"]),
    ("actionscript", &["as3"]),
    ("flex", &["adobeflex"]),
];

fn builtin(head: &str) -> &'static [&'static str] {
    let k = stem(&norm(head));
    ALIASES.iter().find(|(h, _)| stem(h) == k).map_or(&[], |(_, a)| a)
}

// ---------------------------------------------------------------- offline backend

/// Deterministic, no network: exact key, cloud.toml `related`, built-in alias, stem, containment.
pub struct Offline;

impl Scorer for Offline {
    fn assign(&self, heads: &[Word], term: &str) -> Option<Match> {
        let (k, s) = (norm(term), stem(&norm(term)));
        let hit = |why: &str, head: usize| Some(Match { head, why: why.into() });
        if let Some(i) = heads.iter().position(|h| norm(&h.text) == k) {
            return hit("same word", i);
        }
        if let Some(i) = heads.iter().position(|h| h.related.iter().any(|r| norm(r) == k)) {
            return hit("related in cloud.toml", i);
        }
        if let Some(i) = heads.iter().position(|h| builtin(&h.text).iter().any(|a| *a == k || stem(a) == s)) {
            return hit("built-in alias", i);
        }
        if let Some(i) = heads.iter().position(|h| stem(&norm(&h.text)) == s || h.related.iter().any(|r| stem(&norm(r)) == s)) {
            return hit("stem match", i);
        }
        // "TypeScript compiler" -> TypeScript: the longest head whose tokens appear in the term.
        let tt = tokens(term);
        heads
            .iter()
            .enumerate()
            .map(|(i, h)| (i, tokens(&h.text)))
            .filter(|(_, ht)| !ht.is_empty() && tt.windows(ht.len()).any(|w| w == ht.as_slice()))
            .max_by_key(|(_, ht)| ht.len())
            .and_then(|(i, _)| hit("contains the word", i))
    }
}

// ---------------------------------------------------------------- embed backend (opt-in)

/// Offline first; what is left goes by cosine similarity of embeddings. Sends the word list only.
pub struct Embed {
    vecs: BTreeMap<String, Vec<f32>>,
    threshold: f32,
}

impl Embed {
    /// The request: `(endpoint, JSON body, input order)`. Only word texts go out.
    /// POST it with any HTTP client, add `Authorization: Bearer $CLOUD_EMBED_API_KEY` if needed,
    /// then hand the response to [`Embed::from_response`].
    pub fn request(cfg: &Config, heads: &[Word], terms: &[String]) -> Result<(String, Value, Vec<String>)> {
        let endpoint = cfg.endpoint.clone().context("[semantic] endpoint is not set (OpenAI-compatible /v1/embeddings)")?;
        let model = cfg.model.as_deref().context("[semantic] model is not set")?;
        let mut input: Vec<String> = heads.iter().map(|h| h.text.clone()).chain(terms.iter().cloned()).collect();
        input.sort();
        input.dedup();
        let body = json!({ "model": model, "input": input });
        Ok((endpoint, body, input))
    }

    /// Parse an OpenAI-style `{"data": [{"embedding": [..]}, ..]}` response for `input`.
    pub fn from_response(cfg: &Config, input: Vec<String>, body: &Value) -> Result<Self> {
        let data = body["data"].as_array().context("embeddings response has no `data`")?;
        if data.len() != input.len() {
            bail!("asked for {} embeddings, got {}", input.len(), data.len());
        }
        let vecs = input
            .into_iter()
            .zip(data)
            .map(|(t, d)| (t, d["embedding"].as_array().into_iter().flatten().filter_map(Value::as_f64).map(|x| x as f32).collect()))
            .collect();
        Ok(Self { vecs, threshold: cfg.threshold })
    }

    /// Request + parse over HTTP (feature `http`).
    #[cfg(feature = "http")]
    pub fn fetch(cfg: &Config, heads: &[Word], terms: &[String]) -> Result<Self> {
        let (endpoint, body, input) = Self::request(cfg, heads, terms)?;
        let key = std::env::var("CLOUD_EMBED_API_KEY").ok();
        let resp = crate::http::post_json(&endpoint, &body, key.as_deref())?;
        Self::from_response(cfg, input, &resp)
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let n = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (n(a) * n(b)).max(1e-9)
}

impl Scorer for Embed {
    fn assign(&self, heads: &[Word], term: &str) -> Option<Match> {
        if let Some(m) = Offline.assign(heads, term) {
            return Some(m);
        }
        let tv = self.vecs.get(term)?;
        heads
            .iter()
            .enumerate()
            .filter_map(|(i, h)| Some((i, cosine(tv, self.vecs.get(&h.text)?))))
            .filter(|(_, sim)| *sim >= self.threshold)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(head, sim)| Match { head, why: format!("embedding similarity {sim:.2}") })
    }
}

// ---------------------------------------------------------------- weighting

/// One final word and why it weighs what it does.
pub struct Explained {
    pub word: Word,
    pub reasons: Vec<String>,
}

/// Fold `terms` into `heads`, add CV mentions. `new_color` is the resolved colour for new words.
pub fn weigh(
    heads: Vec<Word>,
    terms: &[String],
    corpus: &str,
    scorer: &dyn Scorer,
    cfg: &Config,
    new_color: (&str, &str),
) -> Vec<Explained> {
    // Duplicate heads (same normalised key) merge: max weight, union of related terms.
    let mut out: Vec<Explained> = Vec::new();
    for h in heads {
        match out.iter_mut().find(|e| norm(&e.word.text) == norm(&h.text)) {
            Some(e) => {
                e.word.weight = e.word.weight.max(h.weight);
                e.word.related.extend(h.related);
                e.reasons.push(format!("merged duplicate `{}`", h.text));
            }
            None => out.push(Explained { reasons: vec![format!("base {}", h.weight)], word: h }),
        }
    }

    let mut term_added = vec![0.0f32; out.len()];
    for t in terms.iter().filter(|t| !t.trim().is_empty()) {
        let words: Vec<Word> = out.iter().map(|e| e.word.clone()).collect();
        match scorer.assign(&words, t) {
            Some(m) => {
                let room = (cfg.term_cap - term_added[m.head]).max(0.0);
                let b = cfg.term_boost.min(room);
                term_added[m.head] += b;
                out[m.head].word.weight += b;
                out[m.head].reasons.push(if b > 0.0 {
                    format!("+{b:.2} term `{t}` ({})", m.why)
                } else {
                    format!("term `{t}` ({}, cap reached)", m.why)
                });
            }
            None => {
                out.push(Explained {
                    word: Word {
                        text: t.clone(),
                        weight: cfg.new_weight,
                        color: new_color.0.to_owned(),
                        color_dark: new_color.1.to_owned(),
                        related: Vec::new(),
                    },
                    reasons: vec![format!("new from terms, base {}", cfg.new_weight)],
                });
                term_added.push(0.0);
            }
        }
    }

    // CV mentions, longest phrase first so "React Native" does not also count as "React".
    let toks = tokens(corpus);
    let mut phrases: Vec<(usize, Vec<String>)> = Vec::new();
    for (i, e) in out.iter().enumerate() {
        let w = &e.word;
        let mut ps: Vec<Vec<String>> = std::iter::once(w.text.as_str()).chain(w.related.iter().map(String::as_str)).map(tokens).collect();
        ps.extend(builtin(&w.text).iter().map(|a| vec![stem(a)]));
        phrases.extend(ps.into_iter().filter(|p| !p.is_empty()).map(|p| (i, p)));
    }
    phrases.sort_by_key(|p| std::cmp::Reverse(p.1.len()));
    let mut used = vec![false; toks.len()];
    let mut mentions = vec![0u32; out.len()];
    for (i, p) in &phrases {
        for s in 0..toks.len().saturating_sub(p.len() - 1) {
            if toks[s..s + p.len()] == p[..] && !used[s..s + p.len()].iter().any(|&u| u) {
                used[s..s + p.len()].iter_mut().for_each(|u| *u = true);
                mentions[*i] += 1;
            }
        }
    }
    for (e, n) in out.iter_mut().zip(mentions) {
        if n > 0 {
            let b = cfg.mention_boost * n.min(cfg.mention_cap) as f32;
            e.word.weight += b;
            e.reasons.push(format!("+{b:.2} mentioned {n}x in the corpus"));
        }
    }
    out
}

/// Live boost from GitHub activity: `(term, repos)` pairs - languages and topics only. A term
/// that does not fold into an existing word is ignored, never added: the public cloud only ever
/// shows curated words. Returns the ignored terms (for `--explain`).
pub fn github_boost(out: &mut [Explained], activity: &[(String, u32)], scorer: &dyn Scorer, cfg: &Config) -> Vec<String> {
    let words: Vec<Word> = out.iter().map(|e| e.word.clone()).collect();
    let mut added = vec![0.0f32; out.len()];
    let mut ignored = Vec::new();
    for (term, n) in activity {
        let Some(m) = scorer.assign(&words, term) else {
            ignored.push(term.clone());
            continue;
        };
        let b = (cfg.github_boost * *n as f32).min(cfg.github_cap - added[m.head]).max(0.0);
        added[m.head] += b;
        out[m.head].word.weight += b;
        out[m.head].reasons.push(format!("+{b:.2} github: `{term}` in {n} recent repo(s) ({})", m.why));
    }
    ignored
}

/// All strings of a JSON value, space-separated - the corpus for mention counts.
pub fn corpus(v: &Value) -> String {
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::String(s) => {
                out.push_str(s);
                out.push(' ');
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(m) => m.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut s = String::new();
    walk(v, &mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, weight: f32, related: &[&str]) -> Word {
        Word {
            text: text.into(),
            weight,
            color: "#000000".into(),
            color_dark: "#ffffff".into(),
            related: related.iter().map(|s| (*s).into()).collect(),
        }
    }

    #[test]
    fn normalises_and_stems() {
        assert_eq!(norm("Next.js"), "nextjs");
        assert_eq!(stem(&norm("NextJS")), stem(&norm("next")));
        assert_eq!(stem(&norm("APIs")), "api");
        assert_eq!(norm("C#"), "c#");
    }

    #[test]
    fn folds_related_terms_into_head() {
        let heads =
            vec![w("TypeScript", 10.0, &["typed language"]), w("Rust", 9.0, &[]), w("React", 8.0, &[]), w("React Native", 6.0, &[])];
        let terms: Vec<String> =
            ["typescript", "tsc", "typed language", "NextJS", "nextjs", "TypeScript compiler"].map(String::from).to_vec();
        let out = weigh(heads, &terms, "", &Offline, &Config { term_cap: 9.0, ..Config::default() }, ("#999999", "#666666"));
        let ts = &out[0];
        assert!((ts.word.weight - (10.0 + 4.0 * 0.35)).abs() < 1e-4, "{}", ts.word.weight);
        assert!(ts.reasons.iter().any(|r| r.contains("tsc") && r.contains("built-in")));
        // NextJS is new; the second spelling folds into it.
        let next = out.iter().find(|e| e.word.text == "NextJS").expect("new word");
        assert!((next.word.weight - 3.35).abs() < 1e-4);
    }

    #[test]
    fn mentions_prefer_longer_phrases() {
        let heads = vec![w("React", 8.0, &[]), w("React Native", 6.0, &[]), w("Next.js", 8.0, &[])];
        let out = weigh(heads, &[], "React Native apps, Next.js / React and NextJS", &Offline, &Config::default(), ("#999999", "#666666"));
        let got: Vec<_> = out.iter().map(|e| e.word.weight).collect();
        for (g, want) in got.iter().zip([8.15, 6.15, 8.3]) {
            assert!((g - want).abs() < 1e-4, "{got:?}");
        }
    }

    #[test]
    fn github_only_boosts_curated_words() {
        let heads = vec![w("TypeScript", 10.0, &[]), w("Rust", 9.5, &[])];
        let mut out = weigh(heads, &[], "", &Offline, &Config::default(), ("#999999", "#666666"));
        let act = vec![("Rust".to_owned(), 9), ("some-internal-topic".to_owned(), 3), ("ts".to_owned(), 1)];
        let ignored = github_boost(&mut out, &act, &Offline, &Config::default());
        assert_eq!(out.len(), 2, "never adds words");
        assert_eq!(ignored, vec!["some-internal-topic"]);
        assert!((out[1].word.weight - 10.0).abs() < 1e-4, "capped at +0.5");
        assert!((out[0].word.weight - 10.1).abs() < 1e-4);
    }
}
