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
use std::fmt;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::source::Word;

/// Which [`Scorer`] decides whether a term belongs to a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// [`Offline`]: deterministic, no network.
    #[default]
    Offline,
    /// [`Embed`]: offline first, then embedding similarity. Sends the word list only.
    Embed,
}

impl Backend {
    /// Return the name used in the word list and on the command line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Embed => "embed",
        }
    }
}

impl FromStr for Backend {
    type Err = anyhow::Error;

    fn from_str(name: &str) -> Result<Self> {
        match name {
            "offline" => Ok(Self::Offline),
            "embed" => Ok(Self::Embed),
            other => bail!("unknown semantic backend `{other}` (offline | embed)"),
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Command-line values: the same names as in the word list.
#[cfg(feature = "cli")]
impl clap::ValueEnum for Backend {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::Offline, Self::Embed]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.name()))
    }
}

/// The `[semantic]` table.
#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// `offline` (default) or `embed`, see [`Backend`].
    pub backend: String,
    /// Boost a head word gets per term that folds into it.
    pub term_boost: f32,
    /// Most a head word can gain from terms.
    pub term_cap: f32,
    /// Boost per mention in the corpus.
    pub mention_boost: f32,
    /// Most mentions that count per word.
    pub mention_cap: u32,
    /// Weight for words that only come from `terms`.
    pub new_weight: f32,
    /// Colour name for words that only come from `terms`.
    pub new_color: String,
    /// Embed backend: OpenAI-compatible `/v1/embeddings` URL.
    pub endpoint: Option<String>,
    /// Embed backend: model name.
    pub model: Option<String>,
    /// Embed backend: minimum cosine similarity for a match.
    pub threshold: f32,
    /// GitHub boost per recent repo that uses a language or topic.
    pub github_boost: f32,
    /// Most a word can gain from GitHub activity.
    pub github_cap: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: Backend::Offline.to_string(),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Index into the heads passed to [`Scorer::assign`].
    pub head: usize,
    /// Short reason, shown by `--explain`.
    pub why: String,
}

/// Decide which head word, if any, a term belongs to.
pub trait Scorer {
    /// Return the head `term` folds into, or `None` if it matches no head.
    fn assign(&self, heads: &[Word], term: &str) -> Option<Match>;
}

// ---------------------------------------------------------------- normalisation

/// Normalise a term to a match key: lowercase, alphanumerics plus `#` / `+` only.
///
/// ```
/// assert_eq!(wordcloud::semantic::norm("Next.js"), "nextjs");
/// assert_eq!(wordcloud::semantic::norm("OAuth 2.1"), "oauth21");
/// ```
#[must_use]
pub fn norm(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || *c == '#' || *c == '+')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Stem a normalised key, very lightly: a trailing `js` or plural `s` goes.
///
/// ```
/// assert_eq!(wordcloud::semantic::stem("nextjs"), "next");
/// assert_eq!(wordcloud::semantic::stem("apis"), "api");
/// ```
#[must_use]
pub fn stem(key: &str) -> String {
    let key = key
        .strip_suffix("js")
        .filter(|rest| rest.len() >= 3)
        .unwrap_or(key);
    let key = if key.len() > 3 && key.ends_with('s') && !key.ends_with("ss") {
        &key[..key.len() - 1]
    } else {
        key
    };
    key.to_owned()
}

/// Split a phrase into tokens, each normalised and stemmed: `API-Gateway` -> `api`, `gateway`.
fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || matches!(c, '#' | '+' | '.')))
        .map(|t| stem(&norm(t)))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Built-in aliases, keyed by the normalised head. Only used when that head is in the list.
#[rustfmt::skip]
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
    let key = stem(&norm(head));
    ALIASES
        .iter()
        .find(|(alias_head, _)| stem(alias_head) == key)
        .map_or(&[], |(_, aliases)| aliases)
}

// ---------------------------------------------------------------- offline backend

/// Match terms without a network: exact key, `related` in the word list, built-in alias, stem,
/// then containment ("TypeScript compiler" -> TypeScript).
#[derive(Debug, Clone, Copy, Default)]
pub struct Offline;

impl Scorer for Offline {
    fn assign(&self, heads: &[Word], term: &str) -> Option<Match> {
        let key = norm(term);
        let stemmed = stem(&key);
        let hit = |why: &str, head: usize| {
            Some(Match {
                head,
                why: why.into(),
            })
        };
        if let Some(i) = heads.iter().position(|h| norm(&h.text) == key) {
            return hit("same word", i);
        }
        if let Some(i) = heads
            .iter()
            .position(|h| h.aliases().any(|r| norm(r) == key))
        {
            return hit("related in cloud.toml", i);
        }
        if let Some(i) = heads.iter().position(|h| {
            builtin(&h.text)
                .iter()
                .any(|a| *a == key || stem(a) == stemmed)
        }) {
            return hit("built-in alias", i);
        }
        if let Some(i) = heads.iter().position(|h| {
            stem(&norm(&h.text)) == stemmed || h.aliases().any(|r| stem(&norm(r)) == stemmed)
        }) {
            return hit("stem match", i);
        }
        // "TypeScript compiler" -> TypeScript: the longest head whose tokens appear in the term.
        let term_tokens = tokens(term);
        heads
            .iter()
            .enumerate()
            .map(|(i, h)| (i, tokens(&h.text)))
            .filter(|(_, head_tokens)| {
                !head_tokens.is_empty()
                    && term_tokens
                        .windows(head_tokens.len())
                        .any(|w| w == head_tokens.as_slice())
            })
            .max_by_key(|(_, head_tokens)| head_tokens.len())
            .and_then(|(i, _)| hit("contains the word", i))
    }
}

// ---------------------------------------------------------------- embed backend (opt-in)

/// Match terms offline first; what is left goes by cosine similarity of embeddings. Sends the
/// word list only, never the corpus.
pub struct Embed {
    vectors: BTreeMap<String, Vec<f32>>,
    threshold: f32,
}

impl fmt::Debug for Embed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Embed")
            .field("vectors", &self.vectors.len())
            .field("threshold", &self.threshold)
            .finish()
    }
}

impl Embed {
    /// Build the embeddings request: `(endpoint, JSON body, input order)`. Only word texts go
    /// out.
    ///
    /// POST it with any HTTP client, add `Authorization: Bearer <key>` if the endpoint needs
    /// one, then hand the response to [`Embed::from_response`].
    pub fn request(
        config: &Config,
        heads: &[Word],
        terms: &[String],
    ) -> Result<(String, Value, Vec<String>)> {
        let endpoint = config
            .endpoint
            .clone()
            .context("[semantic] endpoint is not set (OpenAI-compatible /v1/embeddings)")?;
        let model = config
            .model
            .as_deref()
            .context("[semantic] model is not set")?;
        let mut input: Vec<String> = heads
            .iter()
            .map(|h| h.text.clone())
            .chain(terms.iter().cloned())
            .collect();
        input.sort();
        input.dedup();
        let body = json!({ "model": model, "input": input });
        Ok((endpoint, body, input))
    }

    /// Parse an OpenAI-style `{"data": [{"embedding": [..]}, ..]}` response for `input`.
    pub fn from_response(config: &Config, input: Vec<String>, body: &Value) -> Result<Self> {
        let data = body["data"]
            .as_array()
            .context("embeddings response has no `data`")?;
        if data.len() != input.len() {
            bail!("asked for {} embeddings, got {}", input.len(), data.len());
        }
        let vectors = input
            .into_iter()
            .zip(data)
            .map(|(text, item)| {
                let vector = item["embedding"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_f64)
                    .map(|x| x as f32)
                    .collect();
                (text, vector)
            })
            .collect();
        Ok(Self {
            vectors,
            threshold: config.threshold,
        })
    }

    /// Send the request and parse the response (feature `http`). Reads the API key, if any,
    /// from `$CLOUD_EMBED_API_KEY`.
    #[cfg(feature = "http")]
    pub fn fetch(config: &Config, heads: &[Word], terms: &[String]) -> Result<Self> {
        let (endpoint, body, input) = Self::request(config, heads, terms)?;
        let key = std::env::var("CLOUD_EMBED_API_KEY").ok();
        let response = crate::http::post_json(&endpoint, &body, key.as_deref())?;
        Self::from_response(config, input, &response)
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let length = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (length(a) * length(b)).max(1e-9)
}

impl Scorer for Embed {
    fn assign(&self, heads: &[Word], term: &str) -> Option<Match> {
        if let Some(found) = Offline.assign(heads, term) {
            return Some(found);
        }
        let term_vector = self.vectors.get(term)?;
        heads
            .iter()
            .enumerate()
            .filter_map(|(i, h)| Some((i, cosine(term_vector, self.vectors.get(&h.text)?))))
            .filter(|(_, similarity)| *similarity >= self.threshold)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(head, similarity)| Match {
                head,
                why: format!("embedding similarity {similarity:.2}"),
            })
    }
}

// ---------------------------------------------------------------- weighting

/// One final word and why it weighs what it does.
#[derive(Debug, Clone)]
pub struct Explained {
    /// The word with its final weight.
    pub word: Word,
    /// One line per step that changed the weight, starting with the base weight.
    pub reasons: Vec<String>,
}

/// Fold `terms` into `heads` and add corpus mentions. `new_color` is the resolved
/// `(light, dark)` colour for words that only come from `terms`.
#[must_use]
pub fn weigh(
    heads: Vec<Word>,
    terms: &[String],
    corpus: &str,
    scorer: &dyn Scorer,
    config: &Config,
    new_color: (&str, &str),
) -> Vec<Explained> {
    let mut out = merge_duplicates(heads);
    fold_terms(&mut out, terms, scorer, config, new_color);
    count_mentions(&mut out, corpus, config);
    out
}

/// Merge heads with the same normalised key: max weight, union of related terms.
fn merge_duplicates(heads: Vec<Word>) -> Vec<Explained> {
    let mut out: Vec<Explained> = Vec::new();
    for head in heads {
        match out
            .iter_mut()
            .find(|e| norm(&e.word.text) == norm(&head.text))
        {
            Some(e) => {
                e.word.weight = e.word.weight.max(head.weight);
                e.word.related.extend(head.related);
                e.word.variants.extend(head.variants);
                e.reasons.push(format!("merged duplicate `{}`", head.text));
            },
            None => out.push(Explained {
                reasons: vec![format!("base {}", head.weight)],
                word: head,
            }),
        }
    }
    out
}

/// Offer every term to the scorer: a match boosts its head, anything else becomes a new word.
fn fold_terms(
    out: &mut Vec<Explained>,
    terms: &[String],
    scorer: &dyn Scorer,
    config: &Config,
    new_color: (&str, &str),
) {
    let mut term_added = vec![0.0f32; out.len()];
    for term in terms.iter().filter(|t| !t.trim().is_empty()) {
        let words: Vec<Word> = out.iter().map(|e| e.word.clone()).collect();
        if let Some(found) = scorer.assign(&words, term) {
            let room = (config.term_cap - term_added[found.head]).max(0.0);
            let boost = config.term_boost.min(room);
            term_added[found.head] += boost;
            out[found.head].word.weight += boost;
            out[found.head].reasons.push(if boost > 0.0 {
                format!("+{boost:.2} term `{term}` ({})", found.why)
            } else {
                format!("term `{term}` ({}, cap reached)", found.why)
            });
        } else {
            out.push(Explained {
                word: Word {
                    text: term.clone(),
                    weight: config.new_weight,
                    color: new_color.0.to_owned(),
                    color_dark: new_color.1.to_owned(),
                    related: Vec::new(),
                    variants: Vec::new(),
                },
                reasons: vec![format!("new from terms, base {}", config.new_weight)],
            });
            term_added.push(0.0);
        }
    }
}

/// Boost every word per corpus mention, longest phrase first so "React Native" does not also
/// count as "React".
fn count_mentions(out: &mut [Explained], corpus: &str, config: &Config) {
    let corpus_tokens = tokens(corpus);
    let mut phrases: Vec<(usize, Vec<String>)> = Vec::new();
    for (i, e) in out.iter().enumerate() {
        let word = &e.word;
        let spellings = std::iter::once(word.text.as_str())
            .chain(word.aliases().map(String::as_str))
            .map(tokens)
            .chain(builtin(&word.text).iter().map(|a| vec![stem(a)]));
        phrases.extend(spellings.filter(|p| !p.is_empty()).map(|p| (i, p)));
    }
    phrases.sort_by_key(|p| std::cmp::Reverse(p.1.len()));
    let mut used = vec![false; corpus_tokens.len()];
    let mut mentions = vec![0u32; out.len()];
    for (i, phrase) in &phrases {
        for start in 0..corpus_tokens.len().saturating_sub(phrase.len() - 1) {
            let span = start..start + phrase.len();
            if corpus_tokens[span.clone()] == phrase[..] && !used[span.clone()].iter().any(|&u| u) {
                used[span].iter_mut().for_each(|u| *u = true);
                mentions[*i] += 1;
            }
        }
    }
    for (e, count) in out.iter_mut().zip(mentions) {
        if count > 0 {
            let boost = config.mention_boost * count.min(config.mention_cap) as f32;
            e.word.weight += boost;
            e.reasons
                .push(format!("+{boost:.2} mentioned {count}x in the corpus"));
        }
    }
}

/// Boost words from GitHub activity: `(term, repos)` pairs, languages and topics only.
///
/// A term that does not fold into an existing word is ignored, never added: the public cloud
/// only ever shows curated words. Returns the ignored terms (for `--explain`).
pub fn github_boost(
    out: &mut [Explained],
    activity: &[(String, u32)],
    scorer: &dyn Scorer,
    config: &Config,
) -> Vec<String> {
    let words: Vec<Word> = out.iter().map(|e| e.word.clone()).collect();
    let mut added = vec![0.0f32; out.len()];
    let mut ignored = Vec::new();
    for (term, repos) in activity {
        let Some(found) = scorer.assign(&words, term) else {
            ignored.push(term.clone());
            continue;
        };
        let boost = (config.github_boost * *repos as f32)
            .min(config.github_cap - added[found.head])
            .max(0.0);
        added[found.head] += boost;
        out[found.head].word.weight += boost;
        out[found.head].reasons.push(format!(
            "+{boost:.2} github: `{term}` in {repos} recent repo(s) ({})",
            found.why
        ));
    }
    ignored
}

/// Join all strings of a JSON value, space-separated: a corpus for mention counts.
///
/// ```
/// let text = wordcloud::semantic::corpus(&serde_json::json!({ "a": ["Sisu", 3], "b": "Löyly" }));
/// assert_eq!(text, "Sisu Löyly ");
/// ```
#[must_use]
pub fn corpus(value: &Value) -> String {
    fn walk(value: &Value, out: &mut String) {
        match value {
            Value::String(text) => {
                out.push_str(text);
                out.push(' ');
            },
            Value::Array(items) => items.iter().for_each(|x| walk(x, out)),
            Value::Object(map) => map.values().for_each(|x| walk(x, out)),
            _ => {},
        }
    }
    let mut text = String::new();
    walk(value, &mut text);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, weight: f32, related: &[&str]) -> Word {
        Word {
            text: text.into(),
            weight,
            color: "#000000".into(),
            color_dark: "#ffffff".into(),
            related: related.iter().map(|s| (*s).into()).collect(),
            variants: Vec::new(),
        }
    }

    #[test]
    fn norm_and_stem_spellings_share_a_key() {
        assert_eq!(norm("Next.js"), "nextjs");
        assert_eq!(stem(&norm("NextJS")), stem(&norm("next")));
        assert_eq!(stem(&norm("APIs")), "api");
        assert_eq!(norm("C#"), "c#");
    }

    #[test]
    fn weigh_related_terms_fold_into_head() {
        let heads = vec![
            word("TypeScript", 10.0, &["typed language"]),
            word("Rust", 9.0, &[]),
            word("React", 8.0, &[]),
            word("React Native", 6.0, &[]),
        ];
        let terms: Vec<String> = [
            "typescript",
            "tsc",
            "typed language",
            "NextJS",
            "nextjs",
            "TypeScript compiler",
        ]
        .map(String::from)
        .to_vec();
        let config = Config {
            term_cap: 9.0,
            ..Config::default()
        };
        let out = weigh(heads, &terms, "", &Offline, &config, ("#999999", "#666666"));
        let typescript = &out[0];
        assert!(
            (typescript.word.weight - (10.0 + 4.0 * 0.35)).abs() < 1e-4,
            "{}",
            typescript.word.weight
        );
        assert!(
            typescript
                .reasons
                .iter()
                .any(|r| r.contains("tsc") && r.contains("built-in"))
        );
        // NextJS is new; the second spelling folds into it.
        let next = out
            .iter()
            .find(|e| e.word.text == "NextJS")
            .expect("new word");
        assert!((next.word.weight - 3.35).abs() < 1e-4);
    }

    #[test]
    fn weigh_mentions_prefer_longer_phrases() {
        let heads = vec![
            word("React", 8.0, &[]),
            word("React Native", 6.0, &[]),
            word("Next.js", 8.0, &[]),
        ];
        let corpus = "React Native apps, Next.js / React and NextJS";
        let out = weigh(
            heads,
            &[],
            corpus,
            &Offline,
            &Config::default(),
            ("#999999", "#666666"),
        );
        let got: Vec<_> = out.iter().map(|e| e.word.weight).collect();
        for (weight, want) in got.iter().zip([8.15, 6.15, 8.3]) {
            assert!((weight - want).abs() < 1e-4, "{got:?}");
        }
    }

    #[test]
    fn github_boost_unknown_topic_ignored_never_added() {
        let heads = vec![word("TypeScript", 10.0, &[]), word("Rust", 9.5, &[])];
        let config = Config::default();
        let mut out = weigh(heads, &[], "", &Offline, &config, ("#999999", "#666666"));
        let activity = vec![
            ("Rust".to_owned(), 9),
            ("some-other-topic".to_owned(), 3),
            ("ts".to_owned(), 1),
        ];
        let ignored = github_boost(&mut out, &activity, &Offline, &config);
        assert_eq!(out.len(), 2, "never adds words");
        assert_eq!(ignored, vec!["some-other-topic"]);
        assert!((out[1].word.weight - 10.0).abs() < 1e-4, "capped at +0.5");
        assert!((out[0].word.weight - 10.1).abs() < 1e-4);
    }

    #[test]
    fn backend_parse_known_names_roundtrip() {
        for backend in [Backend::Offline, Backend::Embed] {
            assert_eq!(backend.name().parse::<Backend>().expect("known"), backend);
        }
        assert_eq!(Config::default().backend, "offline");
    }

    #[test]
    fn norm_twice_equals_norm_once() {
        for text in [
            "Next.js",
            "OAuth 2.1",
            "C#",
            "Kac moralny",
            "Hätähousu",
            "",
            "  ",
        ] {
            assert_eq!(norm(&norm(text)), norm(text), "{text}");
        }
    }

    #[test]
    fn stem_never_grows_the_key() {
        for key in ["nextjs", "apis", "js", "ss", "class", "sisu", ""] {
            assert!(stem(key).len() <= key.len(), "{key}");
        }
    }
}
