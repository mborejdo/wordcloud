//! `wordcloud`: render a word list (TOML) to a transparent SVG, a dark variant and PNGs.
//!
//! ```sh
//! wordcloud cloud.toml --out cloud.svg --png
//! wordcloud cloud.toml --shape circle --width 1000 --height 1000 --github octocat
//! wordcloud cloud.toml --shape star.png --width 800 --height 800 --fill
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
use wordcloud::{Backend, Cloud, Explained, MaskImage, Options, Source};

/// The command line.
#[derive(Debug, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off command-line flags, not a state machine"
)]
#[command(
    version,
    about = "Word clouds from glyph outlines: transparent SVG and PNG, the same cloud for the same seed"
)]
struct Cli {
    /// Word list, a TOML file.
    #[arg(default_value = "cloud.toml")]
    wordlist: PathBuf,
    /// SVG to write; the dark variant and the PNGs go next to it.
    #[arg(long, default_value = "cloud.svg")]
    out: PathBuf,
    /// Also write PNGs, at this many px per canvas px.
    #[arg(long, value_name = "SCALE", num_args = 0..=1, default_missing_value = "2")]
    png: Option<f32>,
    /// Skip the dark variant.
    #[arg(long)]
    no_dark: bool,
    /// Canvas width in px [default: [style] width, else 800].
    #[arg(long)]
    width: Option<u32>,
    /// Canvas height in px [default: [style] height, else 418].
    #[arg(long)]
    height: Option<u32>,
    /// Layout seed: same list and seed, same cloud [default: [style] seed, else 90].
    #[arg(long)]
    seed: Option<u64>,
    /// TTF or OTF file [default: [style] font, else the bundled Bricolage Grotesque].
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,
    /// Shape the words fill: rect, ellipse, circle, or an image whose dark pixels take words
    /// [default: [style] shape, else rect].
    #[arg(long, alias = "mask", value_name = "NAME|IMAGE")]
    shape: Option<String>,
    /// Flip a shape image: its light pixels take words [default: [style] invert].
    #[arg(long, alias = "mask-invert")]
    invert: bool,
    /// Share of vertical words, 0 to 1 [default: [style] vertical, else 0.18].
    #[arg(long)]
    vertical: Option<f32>,
    /// Fill the gaps with small copies of the lighter words [default: [style] fill].
    #[arg(long)]
    fill: bool,
    /// Boost words by a GitHub user's recent public languages and topics.
    #[arg(long, value_name = "USER")]
    github: Option<String>,
    /// Boost words by their mentions in a text file; it stays local.
    #[arg(long, value_name = "FILE")]
    corpus: Option<PathBuf>,
    /// Semantic backend; embed sends the word list only [default: [semantic] backend].
    #[arg(long, value_enum)]
    scorer: Option<Backend>,
    /// Print the final weights and why, then stop.
    #[arg(long)]
    explain: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let source = Source::load(&cli.wordlist)?;
    let corpus = match &cli.corpus {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
        },
        None => String::new(),
    };
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
    // A shape from the command line is relative to where it runs, one from the list to the list.
    let invert = cli.invert || source.style.invert.unwrap_or(false);
    let mask = match (&cli.shape, &source.style.shape) {
        (Some(shape), _) => Some(MaskImage::open(shape, Path::new(""), &options, invert)?),
        (None, Some(shape)) => Some(MaskImage::open(shape, base, &options, invert)?),
        (None, None) => None,
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
