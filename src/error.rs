//! The error of a `FromStr` in this crate: a colour, a shape or a backend name that does not
//! parse.

use std::fmt;

/// A value from a word list or a command line that does not parse.
///
/// # Examples
///
/// ```
/// let error = "magic".parse::<wordcloud::Backend>().unwrap_err();
/// assert_eq!(error.to_string(), r#"backend: expected offline or embed, got "magic""#);
/// assert_eq!(error.got(), "magic");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    key: &'static str,
    expected: &'static str,
    got: String,
}

impl ParseError {
    /// Build the error for the value `got` of `key`, which takes `expected`.
    pub(crate) fn new(key: &'static str, expected: &'static str, got: &str) -> Self {
        Self {
            key,
            expected,
            got: got.to_owned(),
        }
    }

    /// Return the key the value was for: `colour`, `shape` or `backend`.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        self.key
    }

    /// Return the value as it was written.
    #[must_use]
    pub fn got(&self) -> &str {
        &self.got
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: expected {}, got \"{}\"",
            self.key, self.expected, self.got
        )
    }
}

impl std::error::Error for ParseError {}
