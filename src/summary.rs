//! Utilities for generating and printing review summaries and banners.
//!
//! Functions in this module collate review comments by file path and render a
//! human-readable summary to any writer or directly to stdout. Banner helpers
//! frame output with start and end markers.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Write};

use crate::review_threads::ReviewThread;

/// Banner printed at the start of a code review.
pub const START_BANNER: &str = "========== code review ==========";
/// Banner printed at the end of a code review.
pub const END_BANNER: &str = "========== end of code review ==========";
/// Banner printed before individual review comments.
pub const COMMENTS_BANNER: &str = "======== review comments ========";

/// Produce a count of comments per file path.
///
/// # Examples
///
/// ```
/// use vk::review_threads::{CommentConnection, ReviewComment, ReviewThread};
/// use vk::summary::summarize_files;
///
/// let thread = ReviewThread {
///     comments: CommentConnection { nodes: vec![ReviewComment { path: "a.rs".into(), ..Default::default() }], ..Default::default() },
///     ..Default::default()
/// };
/// let summary = summarize_files(&[thread]);
/// assert_eq!(summary, vec![("a.rs".into(), 1)]);
/// ```
#[must_use]
pub fn summarize_files(threads: &[ReviewThread]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in threads {
        for c in &t.comments.nodes {
            *counts.entry(c.path.clone()).or_default() += 1;
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    // Sort by descending count to surface files with the most discussion.
    // Break ties alphabetically for stable output.
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// Write a preformatted summary to any writer.
///
/// # Errors
///
/// Returns an error if writing to the provided output fails.
///
/// # Examples
///
/// ```
/// use vk::summary::{summarize_files, write_summary};
/// use vk::review_threads::{CommentConnection, ReviewComment, ReviewThread};
///
/// let thread = ReviewThread {
///     comments: CommentConnection { nodes: vec![ReviewComment { path: "a.rs".into(), ..Default::default() }], ..Default::default() },
///     ..Default::default()
/// };
/// let summary = summarize_files(&[thread]);
/// let mut out = Vec::new();
/// write_summary(&mut out, &summary).expect("write summary");
/// assert!(String::from_utf8(out).expect("utf8").contains("a.rs: 1 comment"));
/// ```
pub fn write_summary<W: std::io::Write>(
    mut out: W,
    summary: &[(String, usize)],
) -> std::io::Result<()> {
    if summary.is_empty() {
        return Ok(());
    }
    writeln!(out, "Summary:")?;
    for (path, count) in summary {
        let label = if *count == 1 { "comment" } else { "comments" };
        writeln!(out, "{path}: {count} {label}")?;
    }
    writeln!(out)?;
    Ok(())
}

/// Print the summary directly to stdout.
pub fn print_summary(summary: &[(String, usize)]) {
    if let Err(e) = write_summary(std::io::stdout().lock(), summary) {
        if e.kind() == ErrorKind::BrokenPipe {
            return;
        }
        eprintln!("Failed to write summary: {e}");
    }
}

fn write_banner<W: Write>(mut out: W, text: &str) -> std::io::Result<()> {
    writeln!(out, "{text}")
}

/// Write a banner marking the start of code review output to any writer.
///
/// # Errors
///
/// Returns an error if writing to the provided writer fails.
///
/// # Examples
///
/// ```
/// use vk::summary::write_start_banner;
/// let mut out = Vec::new();
/// write_start_banner(&mut out).expect("write start banner");
/// ```
pub fn write_start_banner<W: Write>(out: W) -> std::io::Result<()> {
    write_banner(out, START_BANNER)
}

/// Print a banner marking the start of code review output.
///
/// # Errors
///
/// Returns an error if writing to stdout fails.
///
/// # Examples
///
/// ```
/// use vk::summary::print_start_banner;
/// print_start_banner().expect("print start banner");
/// ```
pub fn print_start_banner() -> std::io::Result<()> {
    write_start_banner(std::io::stdout().lock())
}

/// Write a banner marking the start of review comments to any writer.
///
/// # Errors
///
/// Returns an error if writing to the provided writer fails.
///
/// # Examples
///
/// ```
/// use vk::summary::write_comments_banner;
/// let mut out = Vec::new();
/// write_comments_banner(&mut out).expect("write comments banner");
/// ```
pub fn write_comments_banner<W: Write>(out: W) -> std::io::Result<()> {
    write_banner(out, COMMENTS_BANNER)
}

/// Print a banner marking the start of review comments.
///
/// # Errors
///
/// Returns an error if writing to stdout fails.
///
/// # Examples
///
/// ```
/// use vk::summary::print_comments_banner;
/// print_comments_banner().expect("print comments banner");
/// ```
pub fn print_comments_banner() -> std::io::Result<()> {
    write_comments_banner(std::io::stdout().lock())
}

/// Write a closing banner once all review threads have been displayed to any
/// writer.
///
/// # Errors
///
/// Returns an error if writing to the provided writer fails.
///
/// # Examples
///
/// ```
/// use vk::summary::write_end_banner;
/// let mut out = Vec::new();
/// write_end_banner(&mut out).expect("write end banner");
/// ```
pub fn write_end_banner<W: Write>(out: W) -> std::io::Result<()> {
    write_banner(out, END_BANNER)
}

/// Print a closing banner once all review threads have been displayed.
///
/// # Errors
///
/// Returns an error if writing to stdout fails.
///
/// # Examples
///
/// ```
/// use vk::summary::print_end_banner;
/// print_end_banner().expect("print end banner");
/// ```
pub fn print_end_banner() -> std::io::Result<()> {
    write_end_banner(std::io::stdout().lock())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review_threads::{CommentConnection, ReviewComment, ReviewThread};

    #[fixture]
    fn review_comment(#[default("test.rs")] path: &str) -> ReviewComment {
        ReviewComment {
            path: path.into(),
            ..Default::default()
        }
    }

    use rstest::*;

    #[rstest]
    #[case(vec![], vec![])]
    #[case(
        vec![ReviewThread {
            comments: CommentConnection {
                nodes: vec![review_comment("a.rs"), review_comment("b.rs")],
                ..Default::default()
            },
            ..Default::default()
        }],
        vec![("a.rs".into(), 1), ("b.rs".into(), 1)]
    )]
    #[case(
        vec![ReviewThread {
            comments: CommentConnection {
                nodes: vec![
                    review_comment("a.rs"),
                    review_comment("b.rs"),
                    review_comment("a.rs"),
                ],
                ..Default::default()
            },
            ..Default::default()
        }],
        vec![("a.rs".into(), 2), ("b.rs".into(), 1)]
    )]
    fn summarize_files_counts_comments(
        #[case] input: Vec<ReviewThread>,
        #[case] expected: Vec<(String, usize)>,
    ) {
        let result = summarize_files(&input);
        assert_eq!(result, expected);
    }

    #[test]
    fn write_summary_outputs_text() {
        let summary = vec![("foo.rs".into(), 1)];
        let mut buf = Vec::new();
        write_summary(&mut buf, &summary).expect("write summary");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("Summary:"));
        assert!(out.contains("foo.rs: 1 comment"));
    }

    #[test]
    fn write_summary_handles_empty() {
        let summary = Vec::<(String, usize)>::new();
        let mut buf = Vec::new();
        write_summary(&mut buf, &summary).expect("write summary");
        assert!(buf.is_empty());
    }

    #[test]
    fn write_start_banner_propagates_io_errors() {
        use std::io::{self, Write};

        struct ErrorWriter;
        impl Write for ErrorWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("Simulated stdout write error"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut writer = ErrorWriter;
        let err = write_start_banner(&mut writer).expect_err("expect error");
        assert_eq!(err.to_string(), "Simulated stdout write error");
    }

    #[test]
    fn write_start_banner_outputs_exact_text() {
        let mut buf = Vec::new();
        write_start_banner(&mut buf).expect("write start banner");
        assert_eq!(
            String::from_utf8(buf).expect("utf8"),
            format!("{START_BANNER}\n"),
        );
    }

    #[test]
    fn write_comments_banner_outputs_exact_text() {
        let mut buf = Vec::new();
        write_comments_banner(&mut buf).expect("write comments banner");
        assert_eq!(
            String::from_utf8(buf).expect("utf8"),
            format!("{COMMENTS_BANNER}\n"),
        );
    }

    #[test]
    fn write_end_banner_propagates_io_errors() {
        use std::io::{self, Write};

        struct ErrorWriter;
        impl Write for ErrorWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("Simulated stdout write error"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut writer = ErrorWriter;
        let err = write_end_banner(&mut writer).expect_err("expect error");
        assert_eq!(err.to_string(), "Simulated stdout write error");
    }

    #[test]
    fn write_end_banner_outputs_exact_text() {
        let mut buf = Vec::new();
        write_end_banner(&mut buf).expect("write end banner");
        assert_eq!(
            String::from_utf8(buf).expect("utf8"),
            format!("{END_BANNER}\n"),
        );
    }
}
