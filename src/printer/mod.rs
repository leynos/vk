//! Helpers for printing review comments and threads.
//!
//! These functions format comments with syntax highlighting using
//! `termimad`. They are separated from the rest of the application so
//! behaviour can be unit tested without capturing stdout.
use termimad::MadSkin;

use crate::diff::format_comment_diff;
use crate::html::collapse_details;
use crate::reviews::PullRequestReview;
use crate::{ReviewComment, ReviewThread};

fn write_author_line<W: std::io::Write>(
    out: &mut W,
    icon: &str,
    login: Option<&str>,
    suffix: &str,
) -> std::io::Result<()> {
    writeln!(
        out,
        "{icon}  \x1b[1m{}\x1b[0m{suffix}",
        login.unwrap_or("(unknown)")
    )
}

fn write_entry_body<W: std::io::Write>(
    out: &mut W,
    skin: &MadSkin,
    icon: &str,
    login: Option<&str>,
    suffix: &str,
    body: &str,
) -> anyhow::Result<()> {
    write_author_line(out, icon, login, suffix)?;
    let body = collapse_details(body);
    skin.write_text_on(out, &body)?;
    writeln!(out)?;
    Ok(())
}

/// Format the body of a single review comment.
///
/// The author's login appears in bold followed by the rendered markdown
/// from the comment body.
///
/// # Examples
///
/// ```ignore
/// use vk::printer::write_comment_body;
/// use vk::ReviewComment;
/// use termimad::MadSkin;
/// let skin = MadSkin::default();
/// let comment = ReviewComment { body: "hello".into(), ..Default::default() };
/// let mut buf = Vec::new();
/// write_comment_body(&mut buf, &skin, &comment).unwrap();
/// ```
pub fn write_comment_body<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    comment: &ReviewComment,
) -> anyhow::Result<()> {
    write_entry_body(
        &mut out,
        skin,
        "\u{1f4ac}",
        comment.author.as_ref().map(|u| u.login.as_str()),
        " wrote:",
        &comment.body,
    )?;
    Ok(())
}

/// Write a single comment including its diff hunk.
///
/// The diff is emitted first, followed by the comment body formatted
/// using [`write_comment_body`].
///
/// # Examples
///
/// ```ignore
/// use vk::printer::write_comment;
/// use vk::ReviewComment;
/// use termimad::MadSkin;
/// let comment = ReviewComment { diff_hunk: "@@ -1 +1 @@\n-old\n+new".into(), ..Default::default() };
/// let mut buf = Vec::new();
/// write_comment(&mut buf, &MadSkin::default(), &comment).unwrap();
/// ```
pub fn write_comment<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    comment: &ReviewComment,
) -> anyhow::Result<()> {
    let diff = format_comment_diff(comment)?;
    write!(out, "{diff}")?;
    write_comment_body(&mut out, skin, comment)?;
    Ok(())
}

/// Write all comments in a thread, showing the diff only once.
///
/// The first comment is printed via [`write_comment`]. Subsequent
/// comments omit the diff and are formatted with [`write_comment_body`].
///
/// # Examples
///
/// ```ignore
/// use vk::printer::write_thread;
/// use vk::{ReviewComment, ReviewThread, CommentConnection};
/// use termimad::MadSkin;
/// let diff = "@@ -1 +1 @@\n-old\n+new\n";
/// let c1 = ReviewComment { diff_hunk: diff.into(), ..Default::default() };
/// let c2 = ReviewComment { diff_hunk: diff.into(), ..Default::default() };
/// let thread = ReviewThread { comments: CommentConnection { nodes: vec![c1,c2], ..Default::default() }, ..Default::default() };
/// let mut buf = Vec::new();
/// write_thread(&mut buf, &MadSkin::default(), &thread).unwrap();
/// ```
pub fn write_thread<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    thread: &ReviewThread,
) -> anyhow::Result<()> {
    let mut iter = thread.comments.nodes.iter();
    if let Some(first) = iter.next() {
        write_comment(&mut out, skin, first)?;
        writeln!(out, "{}", first.url)?;
        for c in iter {
            write_comment_body(&mut out, skin, c)?;
            writeln!(out, "{}", c.url)?;
        }
    }
    Ok(())
}

/// Print reviews to the provided writer using the given skin.
///
/// Each review is printed with the reviewer's login followed by the
/// formatted comment text.
///
/// # Examples
///
/// ```ignore
/// use vk::printer::print_reviews;
/// use vk::reviews::PullRequestReview;
/// use chrono::Utc;
/// use termimad::MadSkin;
/// let review = PullRequestReview { body: "Looks good".into(), submitted_at: Utc::now(), state: "APPROVED".into(), author: None };
/// let mut buf = Vec::new();
/// print_reviews(&mut buf, &MadSkin::default(), &[review]).unwrap();
/// ```
pub fn print_reviews<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    reviews: &[PullRequestReview],
) -> anyhow::Result<()> {
    for r in reviews {
        write_review(&mut out, skin, r)?;
    }
    Ok(())
}

/// Format a single review banner to the provided writer.
///
/// # Examples
///
/// ```ignore
/// use vk::printer::write_review;
/// use vk::reviews::PullRequestReview;
/// use chrono::Utc;
/// use termimad::MadSkin;
/// let review = PullRequestReview { body: "Nice".into(), submitted_at: Utc::now(), state: "APPROVED".into(), author: None };
/// let mut buf = Vec::new();
/// write_review(&mut buf, &MadSkin::default(), &review).unwrap();
/// ```
pub fn write_review<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    review: &PullRequestReview,
) -> anyhow::Result<()> {
    let suffix = format!(" {}:", review.state);
    write_entry_body(
        &mut out,
        skin,
        "\u{1f4dd}",
        review.author.as_ref().map(|u| u.login.as_str()),
        &suffix,
        &review.body,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rstest::rstest;

    use crate::{ReviewComment, User};

    #[test]
    fn print_reviews_formats_authors_and_states() {
        let reviews = [
            PullRequestReview {
                body: "Needs work".into(),
                submitted_at: Utc::now(),
                state: "CHANGES_REQUESTED".into(),
                author: Some(User {
                    login: "alice".into(),
                }),
            },
            PullRequestReview {
                body: "Looks good".into(),
                submitted_at: Utc::now(),
                state: "APPROVED".into(),
                author: None,
            },
        ];
        let skin = MadSkin::default();
        let mut buf = Vec::new();
        print_reviews(&mut buf, &skin, &reviews).expect("print reviews");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("alice"));
        assert!(out.contains("(unknown)"));
        assert!(out.contains("CHANGES_REQUESTED"));
        assert!(out.contains("APPROVED"));
    }

    #[rstest]
    #[case(Some("bob"), "bob", "CHANGES_REQUESTED")]
    #[case(None, "(unknown)", "APPROVED")]
    fn write_review_formats_banner(
        #[case] login: Option<&str>,
        #[case] expected_login: &str,
        #[case] state: &str,
    ) {
        let review = PullRequestReview {
            body: "Nice".into(),
            submitted_at: Utc::now(),
            state: state.into(),
            author: login.map(|l| User { login: l.into() }),
        };
        let mut buf = Vec::new();
        write_review(&mut buf, &MadSkin::default(), &review).expect("write review");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains(expected_login));
        assert!(out.contains(state));
    }

    #[test]
    fn write_review_collapses_details() {
        let review = PullRequestReview {
            body: "<details><summary>sum</summary>hidden</details>".into(),
            submitted_at: Utc::now(),
            state: "APPROVED".into(),
            author: None,
        };
        let mut buf = Vec::new();
        write_review(&mut buf, &MadSkin::default(), &review).expect("write review");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("▶ sum"));
        assert!(!out.contains("hidden"));
    }

    #[rstest]
    #[case(Some("carol"), "carol")]
    #[case(None, "(unknown)")]
    fn write_comment_body_formats_banner(
        #[case] login: Option<&str>,
        #[case] expected_login: &str,
    ) {
        let comment = ReviewComment {
            body: "Hi".into(),
            author: login.map(|l| User { login: l.into() }),
            ..Default::default()
        };
        let mut buf = Vec::new();
        write_comment_body(&mut buf, &MadSkin::default(), &comment).expect("write comment");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains(expected_login));
        assert!(out.contains("wrote"));
    }

    #[test]
    fn write_comment_body_collapses_details() {
        let comment = ReviewComment {
            body: "<details><summary>sum</summary>hidden</details>".into(),
            ..Default::default()
        };
        let mut buf = Vec::new();
        write_comment_body(&mut buf, &MadSkin::default(), &comment).expect("write comment");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("▶ sum"));
        assert!(!out.contains("hidden"));
    }

    #[test]
    fn print_reviews_propagates_writer_errors() {
        struct FailWriter;
        impl std::io::Write for FailWriter {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fail"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let review = PullRequestReview {
            body: "Nice".into(),
            submitted_at: Utc::now(),
            state: "APPROVED".into(),
            author: None,
        };
        let skin = MadSkin::default();
        let err = print_reviews(FailWriter, &skin, &[review]).expect_err("should fail");
        assert!(err.downcast_ref::<std::io::Error>().is_some());
    }
}
