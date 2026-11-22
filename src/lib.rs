//! Word clouds from glyph outlines, as transparent SVG and PNG.
//!
//! 1. Every word is shaped with rustybuzz (kerning, ligatures) and its real glyph outlines are
//!    collected via ttf-parser - no bounding boxes.
//! 2. Per word and size the outline is rasterised (tiny-skia, anti-aliased) into a coverage
//!    bitmap, dilated by half the gap and packed into rows of `u64` bits.
//! 3. Words go in largest first. Each one walks an Archimedean spiral (stretched to the canvas
//!    aspect) from a seeded start near the centre; the first position where its bit sprite does
//!    not intersect the occupancy bitmask wins. The test is a shifted AND over 64 px at a time.
//! 4. An optional shape, a [`Shape`] or any image ([`MaskImage::open`]), pre-fills the
//!    occupancy: blocked pixels take no words.
//! 5. One global scale is found by bisection: the largest scale at which every word fits at its
//!    nominal size. The result is written as SVG glyph paths (no font needed), optionally also
//!    as PNG.
//!
//! Everything random comes from one seeded `SplitMix64`, so a seed always gives the same cloud.
//!
//! # Examples
//!
//! ```
//! let source: wordcloud::Source = r##"
//!     [colours]
//!     ink = "#111318"
//!
//!     [[tier]]
//!     weight = 10
//!     words = ["Kalsarikännit", "Schnapsidee"]
//! "##
//! .parse()?;
//! let (words, _ignored) = wordcloud::weigh(&source, "", &wordcloud::semantic::Offline, &[]);
//! let options = wordcloud::Options::from_style(&source.style);
//! let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &options, None)?;
//! assert!(cloud.dropped.is_empty());
//! assert!(cloud.svg(None, false).contains("<title>Kalsarikännit</title>"));
//! # anyhow::Ok(())
//! ```
//!
//! # Features
//!
//! - `cli` (default): the `wordcloud` binary, together with `http`.
//! - `http` (default): `github::activity` and `semantic::Embed::fetch`.
//!
//! As a library, `default-features = false` drops both.

// The layout converts between pixel counts (`usize`, `u32`) and geometry (`f32`) on every
// step. The values are canvas sizes and font units, far from any precision or range limit.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "pixel <-> geometry conversions on canvas-sized values"
)]
// Same word list + same seed = same cloud, across releases too. These lints suggest
// `mul_add`, `hypot` and `midpoint`, which round differently and would move words.
#![allow(
    clippy::suboptimal_flops,
    clippy::imprecise_flops,
    clippy::manual_midpoint,
    reason = "the float operations stay as they are so layouts stay byte-identical"
)]

pub mod color;
mod error;
pub mod layout;
pub mod semantic;
pub mod source;

#[cfg(feature = "http")]
pub mod github;
#[cfg(feature = "http")]
mod http;

pub use color::Rgb;
pub use error::ParseError;
pub use layout::{Cloud, MaskImage, Options, Shape, generate};
pub use semantic::{Backend, Explained, Scorer};
pub use source::{Source, Style, Word};

/// The bundled font: Bricolage Grotesque 96pt `ExtraBold` (SIL OFL 1.1, see
/// `fonts/OFL-BricolageGrotesque.txt`).
pub const DEFAULT_FONT: &[u8] = include_bytes!("../fonts/BricolageGrotesque96pt-ExtraBold.ttf");

/// Run the semantic step on a whole word list and return the final words, heaviest first.
///
/// `terms` fold into words, `corpus` mentions boost, `activity` (e.g. GitHub languages and
/// topics) boosts curated words only. Also returns the activity terms that matched no word:
/// those are ignored, never added.
#[must_use]
pub fn weigh(
    source: &Source,
    corpus: &str,
    scorer: &dyn Scorer,
    activity: &[(String, u32)],
) -> (Vec<Word>, Vec<String>) {
    let (explained, ignored) = explain(source, corpus, scorer, activity);
    (explained.into_iter().map(|e| e.word).collect(), ignored)
}

/// Run [`weigh`], but keep the reasons for every weight.
#[must_use]
pub fn explain(
    source: &Source,
    corpus: &str,
    scorer: &dyn Scorer,
    activity: &[(String, u32)],
) -> (Vec<Explained>, Vec<String>) {
    let config = &source.semantic;
    let new_color = (
        source.color(Some(&config.new_color)),
        source.color_dark(Some(&config.new_color)),
    );
    let mut out = semantic::weigh(
        source.words(),
        &source.terms,
        corpus,
        scorer,
        config,
        (&new_color.0, &new_color.1),
    );
    let ignored = semantic::github_boost(&mut out, activity, scorer, config);
    out.sort_by(|x, y| y.word.weight.total_cmp(&x.word.weight));
    (out, ignored)
}
