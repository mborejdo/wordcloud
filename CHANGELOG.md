# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow semver; a
change that moves words for the same list and seed counts as breaking.

## Unreleased

### Fixed

- `--no-default-features --features cli` no longer fails to compile: the binary needs `http`
  too and is skipped without it. The library builds as before.
- `Cloud::svg` escapes the background and the fill colours in their attributes: a colour
  holding a quote no longer breaks the SVG. A `#rrggbb` colour is written as before.

### Changed

- Internal clean-up of the layout code; CI lints and tests the build without default features.
  No change to any layout or to the public API.

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
