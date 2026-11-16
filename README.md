# wordcloud

Word clouds that actually fit together. Every word is packed by its real letter shapes, not by
boxes, so the cloud gets dense and stays readable. Light and dark from one layout, transparent
background, one small binary - or a GitHub Action that keeps your profile cloud fresh.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="examples/cloud-dark.svg">
  <img src="examples/cloud.svg" alt="Word cloud of funny German, Polish and Finnish words: Kalsarikännit, Schnapsidee, Kac moralny and more" width="560">
</picture>

<sub>Funny words from German, Polish and Finnish. _Kalsarikännit_: Finnish for drinking at home,
in your underwear, with no plans to go out.</sub>

## Quick start

    cargo install --git https://github.com/mborejdo/wordcloud --locked
    wordcloud cloud.toml --png        # cloud.svg, cloud-dark.svg (+ PNGs)

A word list is a bit of TOML - bigger weight, bigger word:

```toml
[style]
width = 1000
height = 1000
shape = "circle"

[[tier]]
weight = 10
color = "accent"
words = ["Kalsarikännit"]

[[tier]]
weight = 7
color = "ink"
words = ["Sisu", "Feierabend", "Kombinować"]
```

Full example with colours for light and dark: [`examples/cloud.toml`](examples/cloud.toml).

## On your GitHub profile

```yaml
- uses: mborejdo/wordcloud@v1
  with:
    wordlist: cloud.toml
    out: cloud.svg
    seed: ${{ github.run_number }}   # new layout every run
```

Copy [`examples/profile-workflow.yml`](examples/profile-workflow.yml) into your profile repo
(a daily reshuffle, plus a rebuild whenever you edit the list) and add
[`examples/README-snippet.md`](examples/README-snippet.md) to its README.

<details>
<summary>How it works</summary>

1. Each word is shaped with [rustybuzz](https://crates.io/crates/rustybuzz) (kerning, ligatures);
   the glyph outlines come from ttf-parser.
2. The outline is rasterised (tiny-skia) into a coverage bitmap, widened by half the gap and
   packed into rows of `u64` bits.
3. Largest first, each word walks an Archimedean spiral from a seeded start near the centre; the
   first spot where its bits miss everything already placed wins.
4. Optional shape: `--shape circle`, `ellipse`, or any image (`--shape star.png`): its dark
   pixels take words, `--invert` flips that.
   Optional `--fill` (or `[style] fill = true`): a vertical word that finds no room is set
   horizontally, and after the layout small copies of the lighter words fill the gaps, centre
   first, so the canvas is covered edge to edge.
5. One global scale is found by bisection: the largest at which every word fits.
6. Output is glyph paths, so the SVG needs no font. Same list + same seed = same picture.

</details>

<details>
<summary>Weighting and options</summary>

- `terms` fold into the word they belong to (`k8s`, `kubectl` -> Kubernetes): exact match,
  your `related` lists, a built-in alias table, light stemming.
- `--corpus file.txt` boosts words by how often your own text mentions them (stays local).
- `--github <user>` boosts words by the languages/topics of your recent public repos - only
  words already in the list.
- `[semantic] backend = "embed"`: any OpenAI-compatible embeddings endpoint (sends the word list only).
- `--explain` prints every final weight and why.
- Flags: `--width --height --seed --shape --invert --vertical --fill --font --png --no-dark`.
- Any shape: `wordcloud cloud.toml --shape examples/star.png --width 800 --height 800 --fill`,
  or `[style] shape = "star.png"` (relative to the word list). With `--fill`, copies of the
  lighter words trace the shape into its corners.
- A wide banner for a profile README: `[style] width = 1600, height = 600, shape = "rect",
  fill = true`, embedded with `width="100%"`. Copies never outgrow their word, the heavier
  half of the list is never copied, and a copy keeps away from its original.

</details>

## As a library

`default-features = false` drops the CLI and HTTP parts.

```toml
wordcloud = { git = "https://github.com/mborejdo/wordcloud", tag = "v1.1.1", default-features = false }
```

```rust
let source = wordcloud::Source::load("cloud.toml".as_ref())?;
let (words, _) = wordcloud::weigh(&source, "", &wordcloud::semantic::Offline, &[]);
let options = wordcloud::Options::from_style(&source.style);
// A built-in name or an image file; `MaskImage::from_bytes` takes an image from memory.
let mask = wordcloud::MaskImage::open("circle", ".".as_ref(), &options, false)?;
let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &options, Some(&mask))?;
std::fs::write("cloud.svg", cloud.svg(None, false))?;
```

`cargo doc --open` has the rest; every public item is documented.

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --all-features
cargo test --no-default-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo deny check
```

That is what CI runs, the second clippy and test line for the library without the CLI and
HTTP parts. The lints are strict (clippy `pedantic` and `nursery`, no `unwrap` or
`println!` outside tests and the CLI, docs on every public item); an `#[allow]` carries its
reason. Same list + same seed must give the same picture, so a change that moves words is a
breaking change and goes in the [CHANGELOG](CHANGELOG.md).

Releases: bump `version` in `Cargo.toml`, push the tag `vX.Y.Z`. CI checks that they match,
builds the binaries and moves `vX`.

## License

Code: MIT or Apache-2.0. Bundled font: Bricolage Grotesque, SIL Open Font License 1.1.
