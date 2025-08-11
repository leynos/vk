//! String boxing utilities to keep error enums compact.
//!
//! These helpers reduce repetition when converting strings into `Box<str>`,
//! letting code call `.boxed()` without verbose `into_boxed_str` calls.

use std::borrow::Cow;

/// Extension trait to convert string-like types into `Box<str>` without clutter.
pub trait BoxedStr {
    /// Box this value as `Box<str>` without intermediate `String` allocation.
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

impl BoxedStr for Box<str> {
    fn boxed(self) -> Box<str> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::BoxedStr;
    use std::borrow::Cow;

    #[test]
    fn string_is_boxed() {
        assert_eq!(&*String::from("hi").boxed(), "hi");
    }

    #[test]
    fn str_is_boxed() {
        assert_eq!(&*"hi".boxed(), "hi");
    }

    #[test]
    fn cow_borrowed_is_boxed() {
        let cow = Cow::Borrowed("hi");
        assert_eq!(&*cow.boxed(), "hi");
    }

    #[test]
    fn cow_owned_is_boxed() {
        let cow: Cow<'_, str> = Cow::Owned(String::from("hi"));
        assert_eq!(&*cow.boxed(), "hi");
    }

    #[test]
    fn box_identity() {
        let b: Box<str> = "hi".into();
        let ptr = b.as_ref().as_ptr();
        let boxed = b.boxed();
        assert!(std::ptr::eq(ptr, boxed.as_ref().as_ptr()));
    }
}
