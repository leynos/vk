//! String boxing utilities to keep error enums compact.
//!
//! These helpers reduce repetition when converting strings into `Box<str>`,
//! letting code call `.boxed()` without verbose `into_boxed_str` calls.

use std::borrow::Cow;

/// Extension trait to convert string-like types into `Box<str>` without clutter.
pub trait BoxedStr {
    /// Box this value as `Box<str>` without intermediate `String` allocation.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use crate::boxed::BoxedStr;
    ///
    /// let boxed: Box<str> = String::from("hi").boxed();
    /// assert_eq!(&*boxed, "hi");
    /// ```
    #[must_use]
    fn boxed(self) -> Box<str>;
}

impl BoxedStr for String {
    #[inline]
    fn boxed(self) -> Box<str> {
        self.into_boxed_str()
    }
}

impl BoxedStr for &str {
    #[inline]
    fn boxed(self) -> Box<str> {
        self.into()
    }
}

impl BoxedStr for Cow<'_, str> {
    #[inline]
    fn boxed(self) -> Box<str> {
        match self {
            Cow::Borrowed(s) => s.into(),
            Cow::Owned(s) => s.into_boxed_str(),
        }
    }
}

impl BoxedStr for Box<str> {
    #[inline]
    fn boxed(self) -> Box<str> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::BoxedStr;
    use rstest::rstest;
    use std::borrow::Cow;

    #[rstest]
    #[case(String::from("hi"))]
    #[case("hi")]
    #[case(Cow::Borrowed("hi"))]
    #[case(Cow::Owned(String::from("hi")))]
    fn boxes_string_like_inputs(#[case] input: impl BoxedStr) {
        assert_eq!(&*input.boxed(), "hi");
    }

    #[test]
    fn box_identity() {
        let b: Box<str> = "hi".into();
        let ptr = b.as_ref().as_ptr();
        let boxed = b.boxed();
        assert!(std::ptr::eq(ptr, boxed.as_ref().as_ptr()));
    }
}
