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
    assert!(ignored.is_empty());
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
    assert!(!cloud.png(1.0, None, false)?.is_empty());
    Ok(())
}
