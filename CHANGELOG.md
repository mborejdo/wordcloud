# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow semver; a
change that moves words for the same list and seed counts as breaking.

## 3.0.1 - 2026-11-17

### Fixed

- Vertical words no longer frame the cloud at the left and right edge: they go right after the
  heaviest word, and only words up to 40% of the canvas height turn vertical. Layouts with
  `vertical` move.

## 3.0.0 - 2026-11-17

### Added

- `variants` on a word: `{ text = "Linux", variants = ["Debian", "Ubuntu", "Alpine"] }`. `fill`
  copies take the variants in turn, then the word, so gaps show related words. Variants fold
  terms like `related`.

### Changed

- **Breaking (library):** new public field `Word::variants`; `Word` literals add
  `variants: vec![]`. Lists without variants lay out as before.

## 2.0.2 - 2026-11-16

### Fixed

- `fill`: a copy keeps away from its word and all its other copies (not only the last one), and
  is at most 70% of the word's size (it could be a twin). Layouts with `fill` move, others don't.

## 2.0.1 - 2026-11-16

### Fixed

- macOS binary built on macOS 15, since GitHub retired the macOS 14 runner and 2.0.0 shipped no
  binaries. Crate unchanged.

## 2.0.0 - 2026-11-16

### Added

- Any image as the shape: `--shape star.png` or `[style] shape = "star.png"` (relative to the
  list), with `--invert` and `[style] invert`. `--mask` and `--mask-invert` stay as aliases.
- `MaskImage::open` (built-in name or image file) and `MaskImage::from_bytes` (image in memory).

### Changed

- `fill` copies a light word up to four times (was once), down to `min_size`, so a shape fills
  into its corners. Layouts with `fill` move, others don't.
- `Source::lint` no longer reports an unknown shape: a name that isn't built in is an image
  file, checked when opened.
- One wording for CLI help and errors (`read <file>`, `unknown shape ...`).
- Layout code cleaned up; CI lints and tests without default features.

### Fixed

- `--no-default-features --features cli` compiles again: the binary needs `http` too and is
  skipped without it. The library builds as before.
- `Cloud::svg` escapes background and fill colours, so a colour with a quote no longer breaks
  the SVG. `#rrggbb` is written as before.

## 1.1.1 - 2026-11-04

### Changed

- Repo toolchain Rust 1.99.1 (was 1.98.1); the crate still builds on `rust-version` 1.88+.
- `toml_writer` 1.2.0. No layout changes.

## 1.1.0 - 2026-11-03

### Added

- `Shape` (`rect`, `ellipse`, `circle`) and `MaskImage::from_shape`, which can't fail.
- `Backend` (`offline`, `embed`) for the semantic step.
- `Rgb`, a parsed `#rrggbb` colour.
- `Source` implements `FromStr`: a word list from a string.
- `Options` implements `Default`.
- `--fill`, `[style] fill = true`, `Options::fill`: fill the canvas edge to edge. A vertical word
  with no room goes horizontal, then copies of the lighter half (one each, never larger than the
  word) fill the gaps, centre first, away from the original. Made for wide `rect` banners like
  1600x600. Off by default, so layouts are unchanged without it.
- `Placed::copy` marks a fill copy; `Cloud::placed` lists copies after the words.
- `Source::lint` also reports colours that are neither a `[colors]` entry nor `#rrggbb`, an
  unknown `[style] shape` and an unknown `[semantic] backend`.
- `--help` text for `--width`, `--height`, `--seed`, `--mask-invert`.
- Docs on every public item, examples run as tests.

### Changed

- `Cargo.toml` version is `1.0.0`, matching the released tag (it said `0.1.0`, so
  `wordcloud --version` was wrong). The release workflow refuses a mismatched tag.
- `Options` and `Style` have a new `fill` field; struct literals without `..` need it.
- `--shape` and `--scorer` parse into `Shape` and `Backend`. Same values, same error otherwise.
- Strict lints (clippy `pedantic`, `nursery`, `cargo`; no `unwrap`/`expect`/`println!` outside
  tests and the CLI), `cargo deny`, rustdoc with warnings denied, all in CI.

Layouts are byte-identical to 1.0.0: same list and seed, same SVG and PNG.

## 1.0.0 - 2026-11-03

First release: the `wordcloud` crate, CLI and GitHub Action.
