# wordcloud

Word clouds that actually fit together. Every word is packed by its real letter shapes, not by
boxes, so the cloud gets dense and stays readable. Light and dark from one layout, transparent
background, one small binary - or a GitHub Action that keeps your profile cloud fresh.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="examples/cloud-dark.svg">
  <img src="examples/cloud.svg" alt="Word cloud of favourite German, Polish and Finnish words: Kalsarikännit, Schnapsidee, Chrząszcz and more" width="560">
</picture>

<sub>Favourite words from German, Polish and Finnish. _Kalsarikännit_: Finnish for drinking at home,
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
4. Optional shape: `--shape circle|ellipse` or a `--mask` image (dark pixels take words).
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
- Flags: `--width --height --seed --shape --vertical --font --mask --png --no-dark`.

As a library: `default-features = false` drops the CLI and HTTP parts.

```rust
let src = wordcloud::Source::load("cloud.toml".as_ref())?;
let (words, _) = wordcloud::weigh(&src, "", &wordcloud::semantic::Offline, &[]);
let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &wordcloud::Options::from_style(&src.style), None)?;
std::fs::write("cloud.svg", cloud.svg(None, false))?;
```

</details>

## License

Code: MIT or Apache-2.0. Bundled font: Bricolage Grotesque, SIL Open Font License 1.1.
Releases: push `vX.Y.Z`, CI builds the binaries and moves `vX`.
