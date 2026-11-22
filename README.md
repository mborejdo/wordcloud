# wordcloud

A word list in, a word cloud out: words packed by glyph outline, not bounding box, as light
and dark SVG or PNG on a transparent background. A CLI and a GitHub Action.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="examples/cloud-dark.svg">
  <img src="examples/cloud.svg" alt="Word cloud of funny German, Polish and Finnish words: Kalsarikännit, Schnapsidee, Kac moralny and more" width="560">
</picture>

<sub>Funny words from German, Polish and Finnish. _Kalsarikännit_: Finnish for drinking at home,
in your underwear, with no plans to go out.</sub>

## Install

```sh
cargo install --git https://github.com/mborejdo/wordcloud --locked
```

## Usage

```sh
wordcloud cloud.toml                       # cloud.svg and cloud-dark.svg
wordcloud cloud.toml --png                 # the PNGs too
wordcloud cloud.toml --shape circle        # rect, ellipse, circle, or an image like star.png
wordcloud cloud.toml --fill                # small copies of the lighter words fill the gaps
wordcloud cloud.toml --seed 7              # another layout; same list and seed, same cloud
wordcloud cloud.toml --explain             # every final weight and why
```

`wordcloud --help` lists all flags.

## Word list

```toml
[style]                  # what the flags take; a flag wins
width = 1000
height = 1000
shape = "circle"         # or an image path, relative to the list
# also: seed, font, invert, vertical, fill

[[tier]]
weight = 10              # bigger weight, bigger word
colour = "accent"
words = ["Kalsarikännit"]

[[tier]]
weight = 7
colour = "ink"
words = ["Sisu", "Feierabend", { text = "Linux", variants = ["Debian", "Ubuntu"] }]
```

`--fill` copies take a word's `variants` in turn. Full example, with light and dark colours:
[`examples/cloud.toml`](examples/cloud.toml). Keys are kebab-case (`term-boost`,
`colours-dark`); the older `color`, `colors` and snake_case names are still read.

## Shapes

<img src="examples/star-cloud.png" alt="The same words filling a star" width="280" align="right">

```sh
wordcloud cloud.toml --shape examples/star.png --width 560 --height 560 --fill
```

Words go where the image is dark (`--invert` flips it). The image is stretched to the canvas,
so match its proportions. `--fill` adds up to four copies of each lighter word, at most 70% of
its size, so the shape fills into its corners.

## Weights

- `terms` fold into their word (`k8s`, `kubectl` count for Kubernetes) by exact match, your
  `related` lists, a built-in alias table and light stemming.
- `--corpus file.txt` boosts words per mention in your text, which stays local.
- `--github user` boosts words matching the user's recent public languages and topics; it adds no
  words.
- `--backend embed` (`[semantic] backend = "embed"`) matches terms via an OpenAI-compatible
  embeddings endpoint, sending only the word list. An API key, if the endpoint needs one, comes
  from `WORDCLOUD_EMBED_API_KEY` (the older `CLOUD_EMBED_API_KEY` still works).

## On your GitHub profile

```yaml
- uses: mborejdo/wordcloud@v3
  with:
    wordlist: cloud.toml
    out: cloud.svg
    seed: ${{ github.run_number }}   # new layout every run
```

[`examples/profile-workflow.yml`](examples/profile-workflow.yml) reshuffles daily and on list
edits; [`examples/README-snippet.md`](examples/README-snippet.md) goes in the profile README.
Wide banner: `width = 1600`, `height = 600`, `fill = true`.

## Library

```toml
wordcloud = { git = "https://github.com/mborejdo/wordcloud", tag = "v3.0.1", default-features = false }
```

```rust
let source = wordcloud::Source::load("cloud.toml".as_ref())?;
let (words, _) = wordcloud::weigh(&source, "", &wordcloud::semantic::Offline, &[]);
let options = wordcloud::Options::from_style(&source.style);
// A built-in name or an image file; `MaskImage::from_bytes` takes an image from memory.
let shape = wordcloud::MaskImage::open("star.png", ".".as_ref(), &options, false)?;
let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &options, Some(&shape))?;
std::fs::write("cloud.svg", cloud.svg(None, false))?;
```

`default-features = false` drops the CLI and HTTP parts. The rest is in `cargo doc --open`.

## How it works

- Each word is shaped (kerning, ligatures) into glyph outlines.
- The outline becomes a bitmap, widened by half the word gap.
- Largest first, each word spirals out from the centre to the first free spot. A shape marks
  its outside as taken.
- Bisection finds the largest size at which every word fits.
- The SVG is glyph paths, no font needed.

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --all-targets --no-default-features --locked -- -D warnings
cargo test --all-features --locked
cargo test --no-default-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
cargo deny check
```

Moving words for the same list and seed is a breaking change. Release: bump `version` in
`Cargo.toml`, push tag `vX.Y.Z`; CI checks they match, builds binaries and moves `vX`.

## License

[Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. The bundled font, Bricolage
Grotesque in `fonts/`, is SIL OFL 1.1.
