//! Helpers for parsing and formatting diff hunks from review comments.

use regex::Regex;
use std::fmt::Write;
use std::sync::LazyLock;

use crate::ReviewComment;

/// Width of the line number gutter in diff output
pub const GUTTER_WIDTH: usize = 5;

/// Recognize the line-range header at the start of a unified diff hunk.
///
/// The pattern is a literal checked by `hunk_re_compiles`; a build failure would
/// leave `None`, and the hunk would then be read without its header offsets
/// rather than panicking.
static HUNK_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"@@ -(?P<old>\d+)(?:,(?P<old_count>\d+))? \+(?P<new>\d+)(?:,(?P<new_count>\d+))? @@",
    )
    .ok()
});

/// Parse diff lines while tracking their old and new line numbers.
fn parse_diff_lines<'a, I>(
    lines: I,
    mut old_line: Option<i32>,
    mut new_line: Option<i32>,
) -> Vec<(Option<i32>, Option<i32>, String)>
where
    I: Iterator<Item = &'a str>,
{
    let mut parsed = Vec::new();
    for l in lines {
        if l.starts_with('+') {
            parsed.push((None, new_line, l.to_owned()));
            if let Some(ref mut n) = new_line {
                *n += 1;
            }
        } else if l.starts_with('-') {
            parsed.push((old_line, None, l.to_owned()));
            if let Some(ref mut o) = old_line {
                *o += 1;
            }
        } else {
            let text = l.strip_prefix(' ').unwrap_or(l);
            parsed.push((old_line, new_line, format!(" {text}")));
            if let Some(ref mut o) = old_line {
                *o += 1;
            }
            if let Some(ref mut n) = new_line {
                *n += 1;
            }
        }
    }
    parsed
}

/// Format a line number for the fixed-width diff gutter.
fn num_disp(num: i32) -> String {
    let mut s = num.to_string();
    if s.len() > GUTTER_WIDTH {
        let start = s.len() - GUTTER_WIDTH;
        s = s.split_off(start);
    }
    format!("{s:>GUTTER_WIDTH$}")
}

/// Format a diff hunk, annotating line numbers and truncating output.
///
/// The returned string is limited to at most 20 lines centred on the
/// comment's target line where possible.
///
/// # Examples
/// ```ignore
/// use vk::diff::format_comment_diff;
/// # use vk::ReviewComment;
/// let comment = ReviewComment {
///     body: String::new(),
///     diff_hunk: "@@ -1 +1 @@\n-line\n+line".into(),
///     original_position: Some(1),
///     position: Some(1),
///     path: String::new(),
///     url: String::new(),
///     author: None,
/// };
/// let diff = format_comment_diff(&comment).unwrap();
/// assert!(diff.contains("-line"));
/// ```
pub fn format_comment_diff(comment: &ReviewComment) -> Result<String, std::fmt::Error> {
    format_diff_with(comment, HUNK_RE.as_ref())
}

/// Format a diff hunk using `hunk_re` to read the header offsets.
///
/// `None` stands for a pattern that failed to compile: the hunk is then read
/// without its header offsets, leaving the line-number gutter blank. Taking the
/// pattern as a parameter lets tests exercise that fallback.
fn format_diff_with(
    comment: &ReviewComment,
    hunk_re: Option<&Regex>,
) -> Result<String, std::fmt::Error> {
    let diff_lines: Vec<&str> = comment
        .diff_hunk
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .collect();
    let mut lines_iter = diff_lines.iter().copied();
    let Some(header) = lines_iter.next() else {
        return Ok(String::new());
    };

    let lines: Vec<(Option<i32>, Option<i32>, String)> =
        hunk_re.and_then(|re| re.captures(header)).map_or_else(
            || parse_diff_lines(diff_lines.iter().copied(), None, None),
            |caps| {
                let old_start: i32 = caps
                    .name("old")
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0);
                let new_start: i32 = caps
                    .name("new")
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0);
                let _old_count: usize = caps
                    .name("old_count")
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(1);
                let _new_count: usize = caps
                    .name("new_count")
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(1);

                parse_diff_lines(lines_iter, Some(old_start), Some(new_start))
            },
        );

    let target_idx = lines
        .iter()
        .position(|(o, n, _)| comment.original_position == *o || comment.position == *n);
    let (start, end) = target_idx.map_or_else(
        || (0, std::cmp::min(lines.len(), 20)),
        |idx| (idx.saturating_sub(5), std::cmp::min(lines.len(), idx + 6)),
    );

    let mut out = String::new();
    for (o, n, text) in lines.get(start..end).unwrap_or(&[]) {
        // Prefer the new line number, fall back to old, or blanks if neither
        let disp = n.or(*o).map_or_else(|| " ".repeat(GUTTER_WIDTH), num_disp);

        writeln!(&mut out, "{disp}|{text}")?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    //! Tests for diff hunk formatting.

    use super::*;
    use std::fmt::Write;

    #[test]
    fn format_comment_diff_sample() {
        let data = include_str!("../tests/fixtures/review_comment.json");
        let comment: ReviewComment = serde_json::from_str(data).expect("deserialize");
        let diff = format_comment_diff(&comment).expect("diff");
        assert!(diff.contains("-import dataclasses"));
        assert!(diff.contains("import typing"));
    }

    /// Match a header with the hunk pattern, which must have compiled.
    fn hunk_captures(header: &str) -> Option<regex::Captures<'_>> {
        HUNK_RE.as_ref().and_then(|re| re.captures(header))
    }

    #[test]
    fn an_unavailable_hunk_pattern_leaves_the_gutter_blank() {
        let comment = ReviewComment {
            body: String::new(),
            diff_hunk: "@@ -3,2 +5,2 @@\n-old\n+new".to_string(),
            original_position: None,
            position: None,
            path: String::new(),
            url: String::new(),
            author: None,
        };

        let diff = format_diff_with(&comment, None).expect("diff");

        assert!(diff.contains("-old") && diff.contains("+new"));
        assert!(
            diff.lines().all(|line| line.starts_with("     |")),
            "expected a blank gutter on every line: {diff:?}"
        );
    }

    #[test]
    fn hunk_re_compiles() {
        assert!(HUNK_RE.is_some(), "the hunk header pattern must compile");
    }

    #[test]
    fn hunk_re_variants() {
        let caps = hunk_captures("@@ -1 +2 @@").expect("regex");
        assert_eq!(&caps["old"], "1");
        assert!(caps.name("old_count").is_none());
        assert_eq!(&caps["new"], "2");
        assert!(caps.name("new_count").is_none());

        let caps = hunk_captures("@@ -3,4 +5 @@").expect("regex");
        assert_eq!(&caps["old"], "3");
        assert_eq!(caps.name("old_count").expect("old count").as_str(), "4");
        assert_eq!(&caps["new"], "5");
        assert!(caps.name("new_count").is_none());

        let caps = hunk_captures("@@ -7 +8,2 @@").expect("regex");
        assert_eq!(&caps["old"], "7");
        assert!(caps.name("old_count").is_none());
        assert_eq!(&caps["new"], "8");
        assert_eq!(caps.name("new_count").expect("new count").as_str(), "2");
    }

    #[test]
    fn format_comment_diff_invalid_header() {
        let comment = ReviewComment {
            body: String::new(),
            diff_hunk: "not a hunk\n-line1\n+line1".to_string(),
            original_position: None,
            position: None,
            path: String::new(),
            url: String::new(),
            author: None,
        };
        let out = format_comment_diff(&comment).expect("diff");
        assert!(out.contains("-line1"));
        assert!(out.contains("+line1"));
    }

    #[test]
    fn format_comment_diff_caps_output() {
        let mut diff = String::from("@@ -1,30 +1,30 @@\n");
        for i in 0..30 {
            writeln!(&mut diff, " line{i}").expect("write diff line");
        }
        let comment = ReviewComment {
            body: String::new(),
            diff_hunk: diff,
            original_position: None,
            position: None,
            path: String::new(),
            url: String::new(),
            author: None,
        };
        let out = format_comment_diff(&comment).expect("diff");
        assert_eq!(out.lines().count(), 20);
    }
}
