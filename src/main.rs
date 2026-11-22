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
    clippy::print_stderr,
    reason = "the CLI's output is its product, its one-line failure goes to stderr"
)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use wordcloud::semantic::{Embed, Offline, Scorer};
use wordcloud::{Backend, Cloud, Explained, MaskImage, Options, Source};

/// What the top of `--help` says.
const ABOUT: &str = "A TOML word list in, a transparent SVG and PNG out

Words are packed by their glyph outlines on a spiral from the centre, into a rectangle, an \
ellipse, a circle or the dark pixels of an image. The same list and seed give the same cloud.";

const FOOTER: &str = "\
Examples:
  wordcloud cloud.toml --out cloud.svg --png        cloud.svg, cloud-dark.svg and both as PNG
  wordcloud cloud.toml --shape circle --width 1000 --height 1000 --github octocat
  wordcloud cloud.toml --shape star.png --width 800 --height 800 --fill
  wordcloud cloud.toml --explain                    every final weight and why

[style] and [semantic] in the word list set the same values; a flag beats both.
The embed backend reads its API key, if any, from WORDCLOUD_EMBED_API_KEY.

Exit codes:
  0  written, or printed with --explain
  2  the list, font or shape can't be read, the embed request fails, or a file can't be
     written (reason: one line on stderr)";

/// The command line.
#[derive(Debug, Parser)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent on/off command-line flags, not a state machine"
)]
#[command(name = "wordcloud", version, about = ABOUT, after_help = FOOTER)]
struct Cli {
    /// Word list, a TOML file (cloud.toml).
    #[arg(default_value = "cloud.toml")]
    wordlist: PathBuf,
    /// SVG to write; the dark variant and the PNGs go next to it (--out dist/cloud.svg).
    #[arg(long, value_name = "FILE", default_value = "cloud.svg")]
    out: PathBuf,
    /// Also write PNGs, at this many px per canvas px (--png 1).
    #[arg(long, value_name = "SCALE", num_args = 0..=1, default_missing_value = "2")]
    png: Option<f32>,
    /// Skip the dark variant (--no-dark).
    #[arg(long)]
    no_dark: bool,
    /// Canvas width (--width 1000) [default: [style] width, else 800].
    #[arg(long, value_name = "PX")]
    width: Option<u32>,
    /// Canvas height (--height 600) [default: [style] height, else 418].
    #[arg(long, value_name = "PX")]
    height: Option<u32>,
    /// Layout seed: same list and seed, same cloud (--seed 7) [default: [style] seed, else 90].
    #[arg(long, value_name = "N")]
    seed: Option<u64>,
    /// TTF or OTF file (--font Inter.ttf) [default: [style] font, else the bundled Bricolage
    /// Grotesque].
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,
    /// Shape the words fill: rect, ellipse, circle, or an image whose dark pixels take words
    /// (--shape star.png) [default: [style] shape, else rect].
    #[arg(long, alias = "mask", value_name = "NAME|IMAGE")]
    shape: Option<String>,
    /// Flip a shape image: its light pixels take words (--invert) [default: [style] invert].
    #[arg(long, alias = "mask-invert")]
    invert: bool,
    /// Share of vertical words, 0 to 1 (--vertical 0.3) [default: [style] vertical, else 0.18].
    #[arg(long, value_name = "SHARE")]
    vertical: Option<f32>,
    /// Fill the gaps with small copies of the lighter words (--fill) [default: [style] fill].
    #[arg(long)]
    fill: bool,
    /// Boost words by a GitHub user's recent public languages and topics (--github octocat).
    #[arg(long, value_name = "USER")]
    github: Option<String>,
    /// Boost words by their mentions in a text file; it stays local (--corpus notes.txt).
    #[arg(long, value_name = "FILE")]
    corpus: Option<PathBuf>,
    /// Semantic backend; embed sends the word list only (--backend embed) [default: [semantic]
    /// backend].
    #[arg(long, alias = "scorer", value_name = "NAME", value_enum)]
    backend: Option<Backend>,
    /// Print the final weights and why, then stop (--explain).
    #[arg(long)]
    explain: bool,
}

fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("wordcloud: {e:#}");
            ExitCode::from(2)
        },
    }
}

fn run(cli: &Cli) -> Result<u8> {
    let source = Source::load(&cli.wordlist)?;
    let corpus = match &cli.corpus {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
        },
        None => String::new(),
    };
    let backend = match cli.backend {
        Some(backend) => backend,
        None => source
            .semantic
            .backend
            .parse()
            .with_context(|| format!("{}: [semantic]", cli.wordlist.display()))?,
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
            cli,
            &source,
            backend,
            &explained,
            activity.len(),
            ignored.len(),
        );
        return Ok(0);
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
    write_outputs(cli, &cloud)?;
    let copies = cloud.placed.iter().filter(|p| p.copy).count();
    let fill = if copies > 0 {
        format!(" + {copies} fill copies")
    } else {
        String::new()
    };
    println!(
        "{} words{fill}, scale {:.1}px, {} ms",
        cloud.placed.len() - copies,
        cloud.scale,
        started.elapsed().as_millis()
    );
    if !cloud.dropped.is_empty() {
        println!("dropped (no room): {}", cloud.dropped.join(", "));
    }
    Ok(0)
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
        write(svg, cloud.svg(None, *dark).as_bytes())?;
        if let Some(scale) = cli.png {
            write(&svg.with_extension("png"), &cloud.png(scale, None, *dark)?)?;
        }
    }
    Ok(())
}

/// Write one file and print `<path>  <N> KB`.
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
    println!("{}  {} KB", path.display(), bytes.len() / 1024);
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn help() -> String {
        Cli::command().render_long_help().to_string()
    }

    #[test]
    fn cli_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn help_has_examples_and_exit_codes_in_keyboard_characters() {
        let help = help();
        assert!(help.contains("Examples:"));
        assert!(help.contains("Exit codes:"));
        assert!(help.is_ascii());
    }

    #[test]
    fn every_flag_has_an_example() {
        let command = Cli::command();
        let help = help();
        for arg in command.get_arguments() {
            let Some(long) = arg.get_long().filter(|l| !matches!(*l, "help" | "version")) else {
                continue;
            };
            let flag = format!("--{long}");
            let in_flag_help = arg
                .get_help()
                .is_some_and(|h| h.to_string().contains(&flag));
            let in_footer = help
                .lines()
                .filter(|l| l.trim_start().starts_with("wordcloud "))
                .any(|l| l.contains(&flag));
            assert!(in_flag_help || in_footer, "{flag} has no example");
        }
    }

    #[test]
    fn scorer_still_selects_the_backend() {
        let cli = Cli::try_parse_from(["wordcloud", "--scorer", "embed"]).expect("old flag");
        assert_eq!(cli.backend, Some(Backend::Embed));
        assert!(!help().contains("--scorer"), "hidden alias");
    }
}
