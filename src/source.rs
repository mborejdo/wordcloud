//! The word list: a TOML file with `[style]`, `[colours]`, `[colours-dark]`, `[semantic]`,
//! `[[tier]]`, `[[word]]` and `terms`. A commented example: `examples/cloud.toml`.

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::color::Rgb;
use crate::semantic::Backend;

/// The colour name of a word that names none.
const DEFAULT_COLOR: &str = "ink";

/// A parsed word list.
///
/// # Examples
///
/// ```
/// let source: wordcloud::Source = r##"
///     [colours]
///     accent = "#ff4f00"
///
///     [[tier]]
///     weight = 10
///     colour = "accent"
///     words = ["Schnapsidee", { text = "Kac moralny", related = ["hangover"] }]
/// "##
/// .parse()?;
/// let words = source.words();
/// assert_eq!(words[0].color, "#ff4f00");
/// assert_eq!(words[1].related, ["hangover"]);
/// assert!(source.lint().is_empty());
/// # anyhow::Ok(())
/// ```
#[derive(Debug, Deserialize)]
pub struct Source {
    /// Canvas and layout defaults.
    #[serde(default)]
    pub style: Style,
    /// Colour names to `#rrggbb`, for the light variant: `[colours]` (also read as `[colors]`).
    #[serde(default, rename = "colours", alias = "colors")]
    pub colors: BTreeMap<String, String>,
    /// Same names as `[colours]`, for the dark variant: `[colours-dark]` (also read as
    /// `[colors_dark]`).
    #[serde(default, rename = "colours-dark", alias = "colors_dark")]
    pub colors_dark: BTreeMap<String, String>,
    /// The semantic step: term folding, mention and GitHub boosts.
    #[serde(default)]
    pub semantic: crate::semantic::Config,
    /// Groups of words that share a weight and a colour.
    #[serde(default, rename = "tier")]
    pub tiers: Vec<Tier>,
    /// Single entries with their own weight: `[[word]] text = "..", weight = .., related = [..]`.
    #[serde(default, rename = "word")]
    pub extra: Vec<Entry>,
    /// Raw input terms: folded into a head word if related, otherwise clustered into new words.
    #[serde(default)]
    pub terms: Vec<String>,
}

/// The `[style]` table. Every field is optional; [`crate::Options::from_style`] fills the
/// defaults.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct Style {
    /// TTF/OTF path, relative to the word list.
    pub font: Option<String>,
    /// Canvas width in px.
    pub width: Option<u32>,
    /// Canvas height in px.
    pub height: Option<u32>,
    /// Layout seed.
    pub seed: Option<u64>,
    /// Share of vertical words, 0 to 1.
    pub vertical: Option<f32>,
    /// Minimum distance between two words, canvas px.
    pub gap: Option<f32>,
    /// Size contrast: `size = scale * (weight / max) ^ contrast`.
    pub contrast: Option<f32>,
    /// The shape the words fill: `rect`, `ellipse`, `circle` (see [`crate::Shape`]) or an image file,
    /// relative to the word list, whose dark pixels take words.
    pub shape: Option<String>,
    /// Flip a shape image: its light or transparent pixels take words.
    pub invert: Option<bool>,
    /// Fill the canvas edge to edge, see [`crate::Options::fill`].
    pub fill: Option<bool>,
}

/// A `[[tier]]`: words that share a weight and a colour.
#[derive(Debug, Deserialize)]
pub struct Tier {
    /// Weight of every word in the tier.
    pub weight: f32,
    /// Colour name from `[colours]`, or a literal `#rrggbb`: key `colour` (also read as
    /// `color`). Default: `ink`.
    #[serde(default, rename = "colour", alias = "color")]
    pub color: Option<String>,
    /// The words.
    pub words: Vec<TierWord>,
}

/// A tier word: a plain string or `{ text = "..", related = [..] }`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum TierWord {
    /// Just the word.
    Plain(String),
    /// The word plus terms that fold into it.
    Rich {
        /// The word.
        text: String,
        /// Aliases and related terms that fold into this word.
        #[serde(default)]
        related: Vec<String>,
        /// Texts the fill copies take instead of the word: `Linux` -> `Debian`, `Alpine`.
        #[serde(default)]
        variants: Vec<String>,
    },
}

/// A `[[word]]` entry with its own weight.
#[derive(Debug, Deserialize)]
pub struct Entry {
    /// The word.
    pub text: String,
    /// Its weight.
    pub weight: f32,
    /// Colour name from `[colours]`, or a literal `#rrggbb`: key `colour` (also read as
    /// `color`). Default: `ink`.
    #[serde(default, rename = "colour", alias = "color")]
    pub color: Option<String>,
    /// Aliases and related terms that fold into this word.
    #[serde(default)]
    pub related: Vec<String>,
    /// Texts the fill copies take instead of the word.
    #[serde(default)]
    pub variants: Vec<String>,
}

impl FromStr for Source {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        toml::from_str(text).context("parse word list")
    }
}

impl Source {
    /// Read and parse a word list file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
    }

    /// Resolve a colour name for the light variant. `None` means `ink`; a name missing from
    /// `[colours]` is returned as is, so a literal `#rrggbb` works too.
    #[must_use]
    pub fn color(&self, name: Option<&str>) -> String {
        let name = name.unwrap_or(DEFAULT_COLOR);
        self.colors
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    }

    /// Resolve a colour name for the dark variant, falling back to the light colour.
    #[must_use]
    pub fn color_dark(&self, name: Option<&str>) -> String {
        let key = name.unwrap_or(DEFAULT_COLOR);
        self.colors_dark
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.color(name))
    }

    /// Return the head words with resolved colours, in file order (tiers first, then
    /// `[[word]]`).
    #[must_use]
    pub fn words(&self) -> Vec<Word> {
        let tiers = self.tiers.iter().flat_map(|tier| {
            let color = self.color(tier.color.as_deref());
            let color_dark = self.color_dark(tier.color.as_deref());
            tier.words.iter().map(move |word| {
                let (text, related, variants) = match word {
                    TierWord::Plain(text) => (text.clone(), Vec::new(), Vec::new()),
                    TierWord::Rich {
                        text,
                        related,
                        variants,
                    } => (text.clone(), related.clone(), variants.clone()),
                };
                Word {
                    text,
                    weight: tier.weight,
                    color: color.clone(),
                    color_dark: color_dark.clone(),
                    related,
                    variants,
                }
            })
        });
        let extra = self.extra.iter().map(|entry| Word {
            text: entry.text.clone(),
            weight: entry.weight,
            color: self.color(entry.color.as_deref()),
            color_dark: self.color_dark(entry.color.as_deref()),
            related: entry.related.clone(),
            variants: entry.variants.clone(),
        });
        tiers.chain(extra).collect()
    }

    /// Find mistakes in the word list: empty and duplicate (case-insensitive) words, colours
    /// that do not resolve to `#rrggbb`, an unknown semantic backend. One line per finding;
    /// empty when the list is clean. A shape is checked when it is opened.
    #[must_use]
    pub fn lint(&self) -> Vec<String> {
        let mut seen = BTreeMap::new();
        let mut colours = BTreeMap::new();
        let mut findings = Vec::new();
        for word in self.words() {
            if word.text.trim().is_empty() {
                findings.push("empty word".into());
            } else if let Some(previous) = seen.insert(word.text.to_lowercase(), word.text.clone())
            {
                findings.push(format!(
                    "duplicate word \"{}\"; also \"{previous}\"",
                    word.text
                ));
            }
            for colour in [word.color, word.color_dark] {
                colours.entry(colour).or_insert_with(|| word.text.clone());
            }
        }
        for (colour, word) in colours {
            if colour.parse::<Rgb>().is_err() {
                findings.push(format!(
                    "colour: expected a [colours] name or #rrggbb, got \"{colour}\"; first used by \"{word}\""
                ));
            }
        }
        if let Err(e) = self.semantic.backend.parse::<Backend>() {
            findings.push(format!("[semantic]: {e:#}"));
        }
        findings
    }
}

/// A word ready for layout: text, weight and both colours.
#[derive(Clone, Debug)]
pub struct Word {
    /// The text as it is drawn.
    pub text: String,
    /// Relative weight; the heaviest word gets the largest size.
    pub weight: f32,
    /// Fill for the light variant.
    pub color: String,
    /// Fill for the dark variant.
    pub color_dark: String,
    /// Aliases and related terms that fold into this word (semantic step).
    pub related: Vec<String>,
    /// Texts the fill copies take in turn instead of the word ([`crate::Options::fill`]);
    /// they fold terms like `related`.
    pub variants: Vec<String>,
}

impl Word {
    /// `related` and `variants`: every text that folds into this word.
    pub(crate) fn aliases(&self) -> impl Iterator<Item = &String> {
        self.related.iter().chain(&self.variants)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_tiers_and_entries_resolve_in_file_order() {
        let source: Source = r##"
            terms = ["tsc"]
            [colours]
            accent = "#ff4f00"
            [[tier]]
            weight = 10
            colour = "accent"
            words = [{ text = "TypeScript", related = ["TS"] }, "Rust"]
            [[word]]
            text = "rust"
            weight = 3
        "##
        .parse()
        .expect("parse");
        let words = source.words();
        assert_eq!(words[0].color, "#ff4f00");
        assert_eq!(words[0].related, vec!["TS"]);
        assert_eq!(words[2].color, "ink");
        assert_eq!(source.terms, vec!["tsc"]);
        // No [colours-dark]: falls back to the light colour.
        assert_eq!(words[0].color_dark, "#ff4f00");
    }

    #[test]
    fn lint_duplicates_and_unresolved_colour_reported() {
        let source: Source = r##"
            [colours]
            accent = "#ff4f00"
            [[tier]]
            weight = 10
            colour = "accent"
            words = ["Rust"]
            [[word]]
            text = "rust"
            weight = 3
        "##
        .parse()
        .expect("parse");
        let findings = source.lint();
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert!(findings[0].starts_with(r#"duplicate word "rust""#));
        assert!(findings[1].contains(r#"got "ink""#));
    }

    #[test]
    fn parse_old_key_names_still_read() {
        let source: Source = r##"
            [colors]
            accent = "#ff4f00"
            [colors_dark]
            accent = "#ffffff"
            [semantic]
            term_boost = 0.5
            term_cap = 1.5
            mention_boost = 0.25
            mention_cap = 4
            new_weight = 2.5
            new_color = "accent"
            github_boost = 0.75
            github_cap = 2.0
            [[tier]]
            weight = 10
            color = "accent"
            words = ["Rust"]
            [[word]]
            text = "Sisu"
            weight = 3
            color = "accent"
        "##
        .parse()
        .expect("parse");
        let words = source.words();
        assert_eq!(words[0].color, "#ff4f00");
        assert_eq!(words[0].color_dark, "#ffffff");
        assert_eq!(words[1].color, "#ff4f00");
        assert!((source.semantic.term_boost - 0.5).abs() < 1e-6);
        assert!((source.semantic.term_cap - 1.5).abs() < 1e-6);
        assert!((source.semantic.mention_boost - 0.25).abs() < 1e-6);
        assert_eq!(source.semantic.mention_cap, 4);
        assert!((source.semantic.new_weight - 2.5).abs() < 1e-6);
        assert!((source.semantic.github_boost - 0.75).abs() < 1e-6);
        assert!((source.semantic.github_cap - 2.0).abs() < 1e-6);
        assert_eq!(source.semantic.new_color, "accent");
    }

    #[test]
    fn parse_kebab_keys_read() {
        let source: Source = r##"
            [colours-dark]
            ink = "#ffffff"
            [semantic]
            term-boost = 0.5
            term-cap = 2.0
            mention-boost = 0.25
            mention-cap = 3
            new-weight = 4.0
            new-colour = "ink"
            github-boost = 0.2
            github-cap = 1.5
        "##
        .parse()
        .expect("parse");
        let semantic = &source.semantic;
        assert_eq!(source.colors_dark["ink"], "#ffffff");
        assert_eq!(semantic.mention_cap, 3);
        assert_eq!(semantic.new_color, "ink");
        assert!((semantic.new_weight - 4.0).abs() < 1e-6);
        assert!((semantic.github_boost - 0.2).abs() < 1e-6);
    }

    #[test]
    fn lint_unknown_backend_reported() {
        let source: Source = r#"
            [semantic]
            backend = "magic"
        "#
        .parse()
        .expect("parse");
        assert_eq!(
            source.lint(),
            vec![r#"[semantic]: backend: expected offline or embed, got "magic""#]
        );
    }
}
