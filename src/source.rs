//! Word list: the TOML format (`[style]`, `[colors]`, `[colors_dark]`, `[semantic]`, `[[tier]]`,
//! `[[word]]`, `terms`).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Source {
    #[serde(default)]
    pub style: Style,
    #[serde(default)]
    pub colors: BTreeMap<String, String>,
    /// Same names as `colors`, for the dark variant.
    #[serde(default)]
    pub colors_dark: BTreeMap<String, String>,
    #[serde(default)]
    pub semantic: crate::semantic::Config,
    #[serde(default, rename = "tier")]
    pub tiers: Vec<Tier>,
    /// Single entries with their own weight: `[[word]] text = "..", weight = .., related = [..]`.
    #[serde(default, rename = "word")]
    pub extra: Vec<Entry>,
    /// Raw input terms: folded into a head word if related, otherwise clustered into new words.
    #[serde(default)]
    pub terms: Vec<String>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Style {
    pub font: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub seed: Option<u64>,
    pub vertical: Option<f32>,
    pub gap: Option<f32>,
    pub contrast: Option<f32>,
    /// Built-in shape: rect, ellipse, circle.
    pub shape: Option<String>,
}

#[derive(Deserialize)]
pub struct Tier {
    pub weight: f32,
    #[serde(default)]
    pub color: Option<String>,
    pub words: Vec<TierWord>,
}

/// A tier word is a plain string or `{ text = "..", related = [..] }`.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum TierWord {
    Plain(String),
    Rich {
        text: String,
        #[serde(default)]
        related: Vec<String>,
    },
}

#[derive(Deserialize)]
pub struct Entry {
    pub text: String,
    pub weight: f32,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub related: Vec<String>,
}

impl Source {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
    }

    pub fn color(&self, name: Option<&str>) -> String {
        let c = name.unwrap_or("ink");
        self.colors.get(c).cloned().unwrap_or_else(|| c.to_owned())
    }

    pub fn color_dark(&self, name: Option<&str>) -> String {
        let c = name.unwrap_or("ink");
        self.colors_dark.get(c).cloned().unwrap_or_else(|| self.color(name))
    }

    /// Head words with resolved colours, in file order (tiers first, then `[[word]]`).
    pub fn words(&self) -> Vec<Word> {
        let tiers = self.tiers.iter().flat_map(|t| {
            let color = self.color(t.color.as_deref());
            let color_dark = self.color_dark(t.color.as_deref());
            t.words.iter().map(move |w| {
                let (text, related) = match w {
                    TierWord::Plain(s) => (s.clone(), Vec::new()),
                    TierWord::Rich { text, related } => (text.clone(), related.clone()),
                };
                Word { text, weight: t.weight, color: color.clone(), color_dark: color_dark.clone(), related }
            })
        });
        let extra = self.extra.iter().map(|e| Word {
            text: e.text.clone(),
            weight: e.weight,
            color: self.color(e.color.as_deref()),
            color_dark: self.color_dark(e.color.as_deref()),
            related: e.related.clone(),
        });
        tiers.chain(extra).collect()
    }

    /// Duplicates (case-insensitive) and empty entries.
    pub fn lint(&self) -> Vec<String> {
        let mut seen = BTreeMap::new();
        let mut out = Vec::new();
        for w in self.words() {
            if w.text.trim().is_empty() {
                out.push("empty word".into());
            } else if let Some(prev) = seen.insert(w.text.to_lowercase(), w.text.clone()) {
                out.push(format!("duplicate word `{}` (also `{prev}`)", w.text));
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub weight: f32,
    pub color: String,
    pub color_dark: String,
    /// Aliases and related terms that fold into this word (semantic step).
    pub related: Vec<String>,
}
