//! The example word list through the public API: it parses, lints clean and fits completely.

use std::path::Path;

use anyhow::Result;
use wordcloud::semantic::Offline;
use wordcloud::{MaskImage, Options, Shape, Source};

fn example() -> Result<Source> {
    Source::load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/cloud.toml"
    )))
}

#[test]
fn example_word_list_lints_clean() -> Result<()> {
    assert_eq!(example()?.lint(), Vec::<String>::new());
    Ok(())
}

#[test]
fn example_word_list_small_canvas_places_every_word() -> Result<()> {
    let source = example()?;
    let (words, ignored) = wordcloud::weigh(&source, "", &Offline, &[]);
    assert_eq!(ignored.len(), 0, "ignored: {ignored:?}");
    let options = Options {
        width: 400,
        height: 400,
        ..Options::from_style(&source.style)
    };
    let shape: Shape = source.style.shape.as_deref().unwrap_or("rect").parse()?;
    let mask = MaskImage::from_shape(shape, &options);
    let cloud = wordcloud::generate(&words, wordcloud::DEFAULT_FONT, &options, Some(&mask))?;
    assert!(cloud.dropped.is_empty(), "dropped: {:?}", cloud.dropped);
    assert_eq!(cloud.placed.len(), words.len());
    let svg = cloud.svg(None, true);
    assert!(svg.contains("<title>Kalsarikännit</title>"));
    assert!(!svg.contains("<text"), "glyph paths only, no font needed");
    assert_ne!(cloud.png(1.0, None, false)?.len(), 0);
    Ok(())
}

#[test]
fn a_font_file_sets_the_words_like_the_same_bytes_built_in() -> Result<()> {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fonts/BricolageGrotesque96pt-ExtraBold.ttf"
    ));
    let font = wordcloud::read_font(path)?;
    assert_eq!(font, wordcloud::DEFAULT_FONT);
    let missing = wordcloud::read_font(Path::new("no-such-font.ttf")).unwrap_err();
    assert!(
        format!("{missing:#}").starts_with("font: read no-such-font.ttf: "),
        "{missing:#}"
    );
    Ok(())
}
