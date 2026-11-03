# wordcloud

Word clouds from real glyph outlines. Deterministic, transparent SVG (+ dark variant, + PNG),
one small Rust binary or a GitHub Action.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="examples/cloud-dark.svg">
  <img src="examples/cloud.svg" alt="Example word cloud" width="520">
</picture>

## How it works

1. Every word is shaped with [rustybuzz](https://crates.io/crates/rustybuzz) (kerning, ligatures);
   the real glyph outlines come from ttf-parser - no bounding boxes.
2. Per word and size the outline is rasterised (tiny-skia, anti-aliased) into a coverage bitmap,
   dilated by half the gap and packed into rows of `u64` bits.
3. Largest first, each word walks an Archimedean spiral (stretched to the canvas) from a seeded
   start near the centre; the first spot where its bits do not hit the occupancy bitmask wins.
4. Optional shape: `--shape circle|ellipse` or a `--mask` image (dark pixels take words).
5. One global scale is found by bisection: the largest at which every word fits.
6. Output is glyph paths, so the SVG needs no font. Light and dark variants share one layout.

Same word list + same seed = same picture.

## Semantic step

Before layout, the word list is weighed:

- `terms` fold into the word they belong to: `tsc`, `TS`, `typed language` -> TypeScript
  (exact, your `related` lists, a built-in alias table, light stemming like Next.js/NextJS/next).
- `--corpus file.txt` adds a boost per mention in your own text (stays local).
- `--github <user>` adds a small boost from the languages/topics of your most recently pushed
  public repos - only to words already in the list. Nothing new ever appears.
- Optional `[semantic] backend = "embed"`: any OpenAI-compatible embeddings endpoint clusters the
  leftovers. Sends the word list only.

`wordcloud cloud.toml --explain` prints every final weight and why.

## CLI

    cargo install --git https://github.com/mborejdo/wordcloud --locked
    wordcloud cloud.toml --out cloud.svg --png        # cloud.svg, cloud-dark.svg, cloud.png, cloud-dark.png
    wordcloud cloud.toml --shape circle --width 1000 --height 1000 --seed 160
    wordcloud cloud.toml --github mborejdo --explain

The word list format: see [`examples/cloud.toml`](examples/cloud.toml).

## GitHub Action

```yaml
- uses: mborejdo/wordcloud@v1
  with:
    wordlist: cloud.toml
    out: cloud.svg
    png: "2"                      # empty = no PNG
    github-user: ${{ github.repository_owner }}
    # width / height / seed / shape override [style]
```

Downloads the prebuilt binary for the runner (Linux x64, macOS arm64); otherwise builds from source.
A profile README that refreshes every 6 hours: [`examples/profile-workflow.yml`](examples/profile-workflow.yml)
and [`examples/README-snippet.md`](examples/README-snippet.md).

## Library

```rust
let src = wordcloud::Source::load("cloud.toml".as_ref())?;
let (words, _) = wordcloud::weigh(&src, "", &wordcloud::semantic::Offline, &[]);
let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &wordcloud::Options::from_style(&src.style), None)?;
std::fs::write("cloud.svg", cloud.svg(None, false))?;
```

`default-features = false` drops the CLI (clap) and HTTP (ureq) parts.

## Releases

Push a semver tag `vX.Y.Z`: CI builds the binaries, creates the release and moves the major tag
`vX` to it.

## License

Code: MIT or Apache-2.0, at your option. Bundled font: Bricolage Grotesque, SIL Open Font
License 1.1 (`fonts/OFL-BricolageGrotesque.txt`).
