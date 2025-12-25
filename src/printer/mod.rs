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

/// Write a comment permalink followed by a horizontal rule separator.
///
/// The separator ensures the URL is visually associated with the preceding
/// comment when multiple threads are printed back-to-back.
fn write_comment_url_with_separator<W: std::io::Write>(
    out: &mut W,
    url: &str,
) -> std::io::Result<()> {
    writeln!(out, "{url}")?;
    writeln!(out, "---")?;
    Ok(())
}

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

/// Item that can be formatted with an author banner and body.
trait Formattable {
    /// Login for the author, if available.
    fn author_login(&self) -> Option<&str>;
    /// Text content to render below the banner.
    fn body(&self) -> &str;
    /// Icon prefixing the banner.
    fn icon(&self) -> &'static str;
    /// Suffix appended after the author.
    fn suffix(&self) -> String;
}

impl Formattable for ReviewComment {
    fn author_login(&self) -> Option<&str> {
        self.author.as_ref().map(|u| u.login.as_str())
    }

    fn body(&self) -> &str {
        &self.body
    }

    fn icon(&self) -> &'static str {
        "💬"
    }

    fn suffix(&self) -> String {
        " wrote:".to_string()
    }
}

impl Formattable for PullRequestReview {
    fn author_login(&self) -> Option<&str> {
        self.author.as_ref().map(|u| u.login.as_str())
    }

    fn body(&self) -> &str {
        &self.body
    }

    fn icon(&self) -> &'static str {
        "📝"
    }

    fn suffix(&self) -> String {
        format!(" {}:", self.state)
    }
}

/// Write a [`Formattable`] item with a banner and rendered markdown body.
///
/// # Examples
///
/// ```ignore
/// use vk::printer::write_formattable;
/// use vk::ReviewComment;
/// use termimad::MadSkin;
/// let comment = ReviewComment { body: "hi".into(), ..Default::default() };
/// let mut buf = Vec::new();
/// write_formattable(&mut buf, &MadSkin::default(), &comment).unwrap();
/// ```
/// Collapse sequences of more than two newlines into at most two newlines.
fn collapse_excessive_newlines(input: String) -> String {
    if !input.contains("\n\n\n") {
        return input;
    }

    let mut buf = String::with_capacity(input.len());
    let mut newline_count = 0;
    for ch in input.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                buf.push(ch);
            }
        } else {
            newline_count = 0;
            buf.push(ch);
        }
    }
    buf
}

fn write_formattable<W: std::io::Write, T: Formattable>(
    mut out: W,
    skin: &MadSkin,
    item: &T,
) -> anyhow::Result<()> {
    let suffix = item.suffix();
    write_author_line(&mut out, item.icon(), item.author_login(), &suffix)?;
    let collapsed = collapse_details(item.body());
    let collapsed = collapse_excessive_newlines(collapsed);
    let formatted = skin.text(&collapsed, None);
    std::io::Write::write_fmt(&mut out, format_args!("{formatted}"))
        .map_err(anyhow::Error::from)?;
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
    out: W,
    skin: &MadSkin,
    comment: &ReviewComment,
) -> anyhow::Result<()> {
    write_formattable(out, skin, comment)
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
        write_comment_url_with_separator(&mut out, &first.url)?;
        for c in iter {
            write_comment_body(&mut out, skin, c)?;
            write_comment_url_with_separator(&mut out, &c.url)?;
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
    out: W,
    skin: &MadSkin,
    review: &PullRequestReview,
) -> anyhow::Result<()> {
    write_formattable(out, skin, review)
}

#[cfg(test)]
mod tests;
