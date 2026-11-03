//! `wordcloud` - render a word list (TOML) to a transparent SVG (+ dark variant, + PNG).
//!
//!   wordcloud cloud.toml --out cloud.svg --png
//!   wordcloud cloud.toml --shape circle --width 1000 --height 1000 --github mborejdo
//!   wordcloud cloud.toml --explain

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use wordcloud::semantic::{Embed, Offline, Scorer};
use wordcloud::{MaskImage, Options, Source};

#[derive(Parser)]
#[command(version, about = "Word clouds from real glyph outlines - deterministic, transparent SVG/PNG")]
struct Cli {
    /// Word list (TOML: [style], [colors], [colors_dark], [semantic], [[tier]], [[word]], terms).
    #[arg(default_value = "cloud.toml")]
    wordlist: PathBuf,
    /// SVG output. `<stem>-dark.svg` and, with --png, `<stem>.png` / `<stem>-dark.png` go next to it.
    #[arg(long, default_value = "cloud.svg")]
    out: PathBuf,
    /// Also write PNGs (with alpha) at this many px per canvas px.
    #[arg(long, num_args = 0..=1, default_missing_value = "2")]
    png: Option<f32>,
    /// Skip the dark variant.
    #[arg(long)]
    no_dark: bool,
    #[arg(long)]
    width: Option<u32>,
    #[arg(long)]
    height: Option<u32>,
    #[arg(long)]
    seed: Option<u64>,
    /// TTF/OTF (default: [style] font, relative to the word list; else the bundled Bricolage Grotesque).
    #[arg(long)]
    font: Option<PathBuf>,
    /// Shape image: dark, opaque pixels take words.
    #[arg(long, conflicts_with = "shape")]
    mask: Option<PathBuf>,
    #[arg(long)]
    mask_invert: bool,
    /// Built-in shape: rect (default), ellipse, circle (= ellipse; use a square canvas).
    #[arg(long, value_parser = ["rect", "ellipse", "circle"])]
    shape: Option<String>,
    /// Share of vertical words, 0..1.
    #[arg(long)]
    vertical: Option<f32>,
    /// Boost words by a GitHub user's recent public languages/topics ($GITHUB_TOKEN optional).
    #[arg(long, value_name = "USER")]
    github: Option<String>,
    /// Text file whose mentions boost words (e.g. a CV as plain text). Stays local.
    #[arg(long)]
    corpus: Option<PathBuf>,
    /// Semantic backend: offline (default) or embed ([semantic] endpoint + model; sends the word list only).
    #[arg(long, value_parser = ["offline", "embed"])]
    scorer: Option<String>,
    /// Print the final weights and why, then stop.
    #[arg(long)]
    explain: bool,
}

fn main() -> Result<()> {
    let a = Cli::parse();
    let src = Source::load(&a.wordlist)?;
    let corpus = a.corpus.as_ref().map(std::fs::read_to_string).transpose().context("read --corpus")?.unwrap_or_default();
    let backend = a.scorer.clone().unwrap_or_else(|| src.semantic.backend.clone());
    let scorer: Box<dyn Scorer> = match backend.as_str() {
        "offline" => Box::new(Offline),
        "embed" => Box::new(Embed::fetch(&src.semantic, &src.words(), &src.terms)?),
        other => bail!("unknown semantic backend `{other}` (offline | embed)"),
    };
    let activity = match &a.github {
        Some(user) => wordcloud::github::activity(user, 20)?,
        None => Vec::new(),
    };

    let (explained, ignored) = wordcloud::explain(&src, &corpus, scorer.as_ref(), &activity);
    if a.explain {
        println!("backend: {backend}  ({} words, {} raw terms)", explained.len(), src.terms.len());
        if a.github.is_some() {
            println!("github: {} activity term(s), {} ignored (no curated word)", activity.len(), ignored.len());
        }
        println!();
        for e in &explained {
            println!("{:>6.2}  {:<24} {}", e.word.weight, e.word.text, e.reasons.join("; "));
        }
        return Ok(());
    }
    let words: Vec<_> = explained.into_iter().map(|e| e.word).collect();

    let mut opt = Options::from_style(&src.style);
    opt.width = a.width.unwrap_or(opt.width);
    opt.height = a.height.unwrap_or(opt.height);
    opt.seed = a.seed.unwrap_or(opt.seed);
    opt.vertical = a.vertical.map_or(opt.vertical, |v| v.clamp(0.0, 1.0));

    let base = a.wordlist.parent().unwrap_or(".".as_ref());
    let font = match a.font.clone().or_else(|| src.style.font.as_deref().map(|f| base.join(f))) {
        Some(p) => std::fs::read(&p).with_context(|| format!("read {}", p.display()))?,
        None => wordcloud::DEFAULT_FONT.to_vec(),
    };
    let shape = a.shape.clone().or_else(|| src.style.shape.clone());
    let mask = match (&a.mask, &shape) {
        (Some(m), _) => Some(MaskImage::load(m, &opt, a.mask_invert)?),
        (None, Some(s)) if s != "rect" => Some(MaskImage::shape(s, &opt)?),
        _ => None,
    };

    let t = std::time::Instant::now();
    let cloud = wordcloud::generate(&words, &font, &opt, mask.as_ref())?;
    if let Some(dir) = a.out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let stem = a.out.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let mut variants = vec![(a.out.clone(), false)];
    if !a.no_dark {
        variants.push((a.out.with_file_name(format!("{stem}-dark.svg")), true));
    }
    for (svg, dark) in &variants {
        std::fs::write(svg, cloud.svg(None, *dark))?;
        println!("ok  {}", svg.display());
        if let Some(scale) = a.png {
            let png = svg.with_extension("png");
            std::fs::write(&png, cloud.png(scale, None, *dark)?)?;
            println!("ok  {}", png.display());
        }
    }
    println!("    {} words, scale {:.1}px, {} ms", cloud.placed.len(), cloud.scale, t.elapsed().as_millis());
    if !cloud.dropped.is_empty() {
        println!("    dropped (no room): {}", cloud.dropped.join(", "));
    }
    Ok(())
}
