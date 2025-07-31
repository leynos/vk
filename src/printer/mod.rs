//! Helpers for printing review comments and threads.
//!
//! These functions format comments with syntax highlighting using
//! `termimad`. They are separated from the rest of the application so
//! behaviour can be unit tested without capturing stdout.
use termimad::MadSkin;

use crate::html::collapse_details;
use crate::reviews::PullRequestReview;
use crate::{ReviewComment, ReviewThread};

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
    let author = comment.author.as_ref().map_or("", |u| u.login.as_str());
    writeln!(out, "\u{1f4ac}  \x1b[1m{author}\x1b[0m wrote:")?;
    let body = collapse_details(&comment.body);
    let _ = skin.write_text_on(&mut out, &body);
    writeln!(out)?;
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
    let diff = crate::format_comment_diff(comment)?;
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

/// Print the body of a review comment to stdout with the given skin.
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
/// print_reviews(&MadSkin::default(), &[review]);
/// ```
pub fn print_reviews(skin: &MadSkin, reviews: &[PullRequestReview]) {
    for r in reviews {
        if let Err(e) = write_review(std::io::stdout().lock(), skin, r) {
            eprintln!("error printing review: {e}");
        }
    }
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
    let author = review
        .author
        .as_ref()
        .map_or("(unknown)", |u| u.login.as_str());
    writeln!(out, "\u{1f4dd}  \x1b[1m{author}\x1b[0m {}:", review.state)?;
    let body = collapse_details(&review.body);
    if let Err(e) = skin.write_text_on(&mut out, &body) {
        eprintln!("error writing review body: {e}");
    }
    writeln!(out)?;
    Ok(())
}
