//! wordcloud - word clouds from real glyph outlines, deterministic, transparent SVG/PNG.
//!
//! 1. Every word is shaped with rustybuzz (kerning, ligatures) and its real glyph outlines are
//!    collected via ttf-parser - no bounding boxes.
//! 2. Per word and size the outline is rasterised (tiny-skia, anti-aliased) into a coverage
//!    bitmap, dilated by half the gap and packed into rows of `u64` bits.
//! 3. Words go in largest first. Each one walks an Archimedean spiral (stretched to the canvas
//!    aspect) from a seeded start near the centre; the first position where its bit sprite does
//!    not intersect the occupancy bitmask wins. The test is a shifted AND over 64 px at a time.
//! 4. An optional mask image pre-fills the occupancy: light or transparent pixels are blocked.
//! 5. One global scale is found by bisection: the largest scale at which every word fits at its
//!    nominal size. The result is written as SVG glyph paths (crisp in typst, no font needed),
//!    optionally also as PNG.
//!
//! Everything random comes from one seeded SplitMix64, so a seed always gives the same cloud.

//!
//! ```no_run
//! let src = wordcloud::Source::load("cloud.toml".as_ref())?;
//! let (words, _ignored) = wordcloud::weigh(&src, "", &wordcloud::semantic::Offline, &[]);
//! let opt = wordcloud::Options::from_style(&src.style);
//! let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &opt, None)?;
//! std::fs::write("cloud.svg", cloud.svg(None, false))?;
//! # anyhow::Ok(())
//! ```

pub mod layout;
pub mod semantic;
pub mod source;

#[cfg(feature = "http")]
pub mod github;
#[cfg(feature = "http")]
mod http;

pub use layout::{Cloud, MaskImage, Options, generate};
pub use semantic::{Explained, Scorer};
pub use source::{Source, Style, Word};

/// Bricolage Grotesque 96pt ExtraBold (SIL OFL 1.1, see fonts/OFL-BricolageGrotesque.txt).
pub const DEFAULT_FONT: &[u8] = include_bytes!("../fonts/BricolageGrotesque96pt-ExtraBold.ttf");

/// The semantic step for a whole word list: `terms` fold into words, `corpus` mentions boost,
/// `activity` (e.g. GitHub languages/topics) boosts curated words only. Sorted by weight, heaviest
/// first. Also returns the activity terms that matched no word (ignored, never added).
pub fn weigh(src: &Source, corpus: &str, scorer: &dyn Scorer, activity: &[(String, u32)]) -> (Vec<Word>, Vec<String>) {
    let (explained, ignored) = explain(src, corpus, scorer, activity);
    (explained.into_iter().map(|e| e.word).collect(), ignored)
}

/// Like [`weigh`], but keeps the reasons for every weight.
pub fn explain(src: &Source, corpus: &str, scorer: &dyn Scorer, activity: &[(String, u32)]) -> (Vec<Explained>, Vec<String>) {
    let cfg = &src.semantic;
    let new_color = (src.color(Some(&cfg.new_color)), src.color_dark(Some(&cfg.new_color)));
    let mut out = semantic::weigh(src.words(), &src.terms, corpus, scorer, cfg, (&new_color.0, &new_color.1));
    let ignored = semantic::github_boost(&mut out, activity, scorer, cfg);
    out.sort_by(|x, y| y.word.weight.total_cmp(&x.word.weight));
    (out, ignored)
}
