//! `wordcloud`: render a word list (TOML) to a transparent SVG, a dark variant and PNGs.
//!
//! ```sh
//! wordcloud cloud.toml --out cloud.svg --png
//! wordcloud cloud.toml --shape circle --width 1000 --height 1000 --github octocat
//! wordcloud cloud.toml --shape rect --width 1600 --height 600 --fill
//! wordcloud cloud.toml --explain
//! ```

#![allow(
    clippy::print_stdout,
    reason = "the CLI's output is its product, not logging"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use wordcloud::semantic::{Embed, Offline, Scorer};
use wordcloud::{Backend, Cloud, Explained, MaskImage, Options, Shape, Source};

/// The command line.
#[derive(Debug, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off command-line flags, not a state machine"
)]
#[command(
    version,
    about = "Word clouds from real glyph outlines - deterministic, transparent SVG/PNG"
)]
struct Cli {
    #[arg(
        default_value = "cloud.toml",
        help = "Word list (TOML: [style], [colors], [colors_dark], [semantic], [[tier]], [[word]], terms)"
    )]
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
    /// Canvas width in px (default: [style] width, else 800).
    #[arg(long)]
    width: Option<u32>,
    /// Canvas height in px (default: [style] height, else 418).
    #[arg(long)]
    height: Option<u32>,
    /// Layout seed (default: [style] seed). Same list + same seed = same picture.
    #[arg(long)]
    seed: Option<u64>,
    /// TTF/OTF (default: [style] font, relative to the word list; else the bundled Bricolage Grotesque).
    #[arg(long)]
    font: Option<PathBuf>,
    /// Shape image: dark, opaque pixels take words.
    #[arg(long, conflicts_with = "shape")]
    mask: Option<PathBuf>,
    /// Flip the mask: light or transparent pixels take words.
    #[arg(long)]
    mask_invert: bool,
    /// Built-in shape (default: [style] shape, else rect). circle = ellipse; use a square canvas.
    #[arg(long, value_enum)]
    shape: Option<Shape>,
    /// Share of vertical words, 0..1.
    #[arg(long)]
    vertical: Option<f32>,
    /// Fill the canvas edge to edge: small copies of the lighter words go into the gaps
    /// (default: [style] fill, else off).
    #[arg(long)]
    fill: bool,
    #[arg(
        long,
        value_name = "USER",
        help = "Boost words by a GitHub user's recent public languages/topics ($GITHUB_TOKEN optional)"
    )]
    github: Option<String>,
    /// Text file whose mentions boost words (e.g. a CV as plain text). Stays local.
    #[arg(long)]
    corpus: Option<PathBuf>,
    /// Semantic backend (default: [semantic] backend). embed sends the word list only.
    #[arg(long, value_enum)]
    scorer: Option<Backend>,
    /// Print the final weights and why, then stop.
    #[arg(long)]
    explain: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let source = Source::load(&cli.wordlist)?;
    let corpus = cli
        .corpus
        .as_ref()
        .map(std::fs::read_to_string)
        .transpose()
        .context("read --corpus")?
        .unwrap_or_default();
    let backend = match cli.scorer {
        Some(backend) => backend,
        None => source.semantic.backend.parse()?,
    };
    let scorer: Box<dyn Scorer> = match backend {
        Backend::Offline => Box::new(Offline),
        Backend::Embed => Box::new(Embed::fetch(
            &source.semantic,
            &source.words(),
            &source.terms,
        )?),
    };
    let activity = match &cli.github {
        Some(user) => wordcloud::github::activity(user, 20)?,
        None => Vec::new(),
    };

    let (explained, ignored) = wordcloud::explain(&source, &corpus, scorer.as_ref(), &activity);
    if cli.explain {
        print_explanation(
            &cli,
            &source,
            backend,
            &explained,
            activity.len(),
            ignored.len(),
        );
        return Ok(());
    }
    let words: Vec<_> = explained.into_iter().map(|e| e.word).collect();

    let mut options = Options::from_style(&source.style);
    options.width = cli.width.unwrap_or(options.width);
    options.height = cli.height.unwrap_or(options.height);
    options.seed = cli.seed.unwrap_or(options.seed);
    options.vertical = cli.vertical.map_or(options.vertical, |v| v.clamp(0.0, 1.0));
    options.fill |= cli.fill;

    let base = cli.wordlist.parent().unwrap_or_else(|| Path::new("."));
    let font = match cli
        .font
        .clone()
        .or_else(|| source.style.font.as_deref().map(|f| base.join(f)))
    {
        Some(path) => std::fs::read(&path).with_context(|| format!("read {}", path.display()))?,
        None => wordcloud::DEFAULT_FONT.to_vec(),
    };
    let shape = match cli.shape {
        Some(shape) => Some(shape),
        None => source.style.shape.as_deref().map(str::parse).transpose()?,
    };
    let mask = match (&cli.mask, shape) {
        (Some(path), _) => Some(MaskImage::load(path, &options, cli.mask_invert)?),
        (None, Some(shape)) if shape != Shape::Rect => Some(MaskImage::from_shape(shape, &options)),
        _ => None,
    };

    let started = std::time::Instant::now();
    let cloud = wordcloud::generate(&words, &font, &options, mask.as_ref())?;
    write_outputs(&cli, &cloud)?;
    let copies = cloud.placed.iter().filter(|p| p.copy).count();
    let fill = if copies > 0 {
        format!(" + {copies} fill copies")
    } else {
        String::new()
    };
    println!(
        "    {} words{fill}, scale {:.1}px, {} ms",
        cloud.placed.len() - copies,
        cloud.scale,
        started.elapsed().as_millis()
    );
    if !cloud.dropped.is_empty() {
        println!("    dropped (no room): {}", cloud.dropped.join(", "));
    }
    Ok(())
}

/// `--explain`: every final weight and the steps that made it.
fn print_explanation(
    cli: &Cli,
    source: &Source,
    backend: Backend,
    explained: &[Explained],
    activity: usize,
    ignored: usize,
) {
    println!(
        "backend: {backend}  ({} words, {} raw terms)",
        explained.len(),
        source.terms.len()
    );
    if cli.github.is_some() {
        println!("github: {activity} activity term(s), {ignored} ignored (no curated word)");
    }
    println!();
    for e in explained {
        println!(
            "{:>6.2}  {:<24} {}",
            e.word.weight,
            e.word.text,
            e.reasons.join("; ")
        );
    }
}

/// Write the light SVG, the dark one unless `--no-dark`, and a PNG next to each with `--png`.
fn write_outputs(cli: &Cli, cloud: &Cloud) -> Result<()> {
    if let Some(dir) = cli.out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let stem = cli
        .out
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut variants = vec![(cli.out.clone(), false)];
    if !cli.no_dark {
        variants.push((cli.out.with_file_name(format!("{stem}-dark.svg")), true));
    }
    for (svg, dark) in &variants {
        std::fs::write(svg, cloud.svg(None, *dark))
            .with_context(|| format!("write {}", svg.display()))?;
        println!("ok  {}", svg.display());
        if let Some(scale) = cli.png {
            let png = svg.with_extension("png");
            std::fs::write(&png, cloud.png(scale, None, *dark)?)
                .with_context(|| format!("write {}", png.display()))?;
            println!("ok  {}", png.display());
        }
    }
    Ok(())
}
