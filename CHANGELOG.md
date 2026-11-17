# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow semver; a
change that moves words for the same list and seed counts as breaking.

## Unreleased

### Added

- `variants` on a word: `{ text = "Linux", variants = ["Debian", "Ubuntu", "Alpine"] }`. The
  `fill` copies take the variants in turn, then the word itself, so the gaps show related
  words instead of the same word again. Variants fold terms like `related`.

### Changed

- **Breaking (library):** `Word` has a new public field `variants`; code that builds a `Word`
  literal adds `variants: vec![]`. Lists without variants lay out as before.

## 2.0.2 - 2026-11-16

### Fixed

- `fill`: a copy keeps away from its word and from all its other copies (it only kept away
  from the last one, so two could sit side by side), and is at most 70% of the word's size
  (it could be a twin). Layouts with `fill` move; layouts without it do not.

## 2.0.1 - 2026-11-16

### Fixed

- The release builds the macOS binary on macOS 15: GitHub retired the macOS 14 runner, so
  2.0.0 has no binaries. No change to the crate.

## 2.0.0 - 2026-11-16

### Added

- Any image as the shape: `--shape star.png` and `[style] shape = "star.png"` (relative to the
  word list) beside the built-in names, `--invert` and `[style] invert`. `--mask` and
  `--mask-invert` stay as aliases.
- `MaskImage::open` (a built-in name or an image file) and `MaskImage::from_bytes` (an image
  from memory).

### Changed

- `fill` copies a light word up to four times (was once), down to `min_size`, so a shape is
  traced into its corners. Layouts with `fill` move; layouts without it do not.
- `Source::lint` no longer reports an unknown shape: a name that is not built in is an image
  file, checked when the shape is opened.
- One wording for the command line's help and errors (`read <file>`, `unknown shape ...`).
- Internal clean-up of the layout code; CI lints and tests the build without default features.

### Fixed

- `--no-default-features --features cli` no longer fails to compile: the binary needs `http`
  too and is skipped without it. The library builds as before.
- `Cloud::svg` escapes the background and the fill colours in their attributes: a colour
  holding a quote no longer breaks the SVG. A `#rrggbb` colour is written as before.

## 1.1.1 - 2026-11-04

### Changed

- Toolchain for working on the repo: Rust 1.99.1 (was 1.98.1). The crate still builds on
  `rust-version` 1.88 and newer.
- Dependency update (`toml_writer` 1.2.0). No change to any layout.

## 1.1.0 - 2026-11-03

### Added

- `Shape` (`rect`, `ellipse`, `circle`) and `MaskImage::from_shape`, which cannot fail.
- `Backend` (`offline`, `embed`) for the semantic step.
- `Rgb`, a parsed `#rrggbb` colour.
- `Source` implements `FromStr`, so a word list can come from a string.
- `Options` implements `Default`.
- `--fill`, `[style] fill = true` and `Options::fill`: fill the canvas edge to edge. A vertical
  word that finds no room is set horizontally instead, and after the layout copies of the
  lighter half of the words (at most one each, never larger than the word) fill the gaps,
  centre first and away from the original. Made for a wide `rect` banner, e.g. 1600x600.
  Off by default: without it every layout is unchanged.
- `Placed::copy` marks a fill copy; `Cloud::placed` lists the copies after the words.
- `Source::lint` also reports colours that resolve to neither a `[colors]` entry nor
  `#rrggbb`, an unknown `[style] shape` and an unknown `[semantic] backend`.
- `--help` text for `--width`, `--height`, `--seed` and `--mask-invert`.
- Docs on every public item, with examples that run as tests.

### Changed

- `Cargo.toml` version is now `1.0.0`, matching the released tag (it said `0.1.0`, so
  `wordcloud --version` was wrong). The release workflow refuses a tag that does not match.
- `Options` and `Style` have a new `fill` field, so a struct literal without `..` needs it.
- `--shape` and `--scorer` are parsed into `Shape` and `Backend`. Same accepted values, same
  error for anything else.
- Strict lints (clippy `pedantic`, `nursery`, `cargo`, no `unwrap`/`expect`/`println!` outside
  tests and the CLI), `cargo deny` and rustdoc with warnings denied, all in CI.

Layouts are byte-identical to 1.0.0: same list + same seed = same SVG and PNG.

## 1.0.0 - 2026-11-03

First release: the `wordcloud` crate, CLI and GitHub Action.
