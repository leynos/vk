//! Helpers for boxing common values.
//!
//! These utilities reduce repetition when converting strings into `Box<str>`,
//! keeping the `VkError` enum compact without verbose calls to
//! `into_boxed_str`.

use std::borrow::Cow;

/// Convert string-like values into `Box<str>`.
pub trait BoxedStr {
    /// Consume `self` and return an owned boxed string.
    fn boxed(self) -> Box<str>;
}

impl BoxedStr for String {
    fn boxed(self) -> Box<str> {
        self.into_boxed_str()
    }
}

impl BoxedStr for &str {
    fn boxed(self) -> Box<str> {
        self.into()
    }
}

impl BoxedStr for Cow<'_, str> {
    fn boxed(self) -> Box<str> {
        match self {
            Cow::Borrowed(s) => s.into(),
            Cow::Owned(s) => s.into_boxed_str(),
        }
    }
}
