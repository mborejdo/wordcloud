//! Colours: the `#rrggbb` values a word list resolves its colour names to.

use std::fmt;
use std::str::FromStr;

use anyhow::{Context, bail};

/// An sRGB colour, written `#rrggbb`.
///
/// # Examples
///
/// ```
/// let orange: wordcloud::Rgb = "#ff4f00".parse()?;
/// assert_eq!((orange.red, orange.green, orange.blue), (0xff, 0x4f, 0x00));
/// assert_eq!(orange.to_string(), "#ff4f00");
/// assert!("ink".parse::<wordcloud::Rgb>().is_err());
/// # anyhow::Ok(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

impl FromStr for Rgb {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits = text.trim_start_matches('#');
        if digits.len() != 6 {
            bail!("colour `{text}` is not #rrggbb");
        }
        let value = u32::from_str_radix(digits, 16).with_context(|| format!("colour `{text}`"))?;
        let [_, red, green, blue] = value.to_be_bytes();
        Ok(Self { red, green, blue })
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_without_hash_accepts_six_digits() {
        let colour: Rgb = "111318".parse().expect("valid");
        assert_eq!(
            colour,
            Rgb {
                red: 0x11,
                green: 0x13,
                blue: 0x18
            }
        );
    }

    #[test]
    fn parse_short_or_named_colour_errors() {
        for text in ["#fff", "ink", "", "#gggggg"] {
            assert!(text.parse::<Rgb>().is_err(), "{text}");
        }
    }

    #[test]
    fn display_then_parse_roundtrips() {
        for (red, green, blue) in [
            (0, 0, 0),
            (255, 255, 255),
            (0x11, 0x13, 0x18),
            (0xff, 0x4f, 0x00),
        ] {
            let colour = Rgb { red, green, blue };
            assert_eq!(
                colour.to_string().parse::<Rgb>().expect("own output"),
                colour
            );
        }
    }
}
