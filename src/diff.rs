//! Helpers for parsing and formatting diff hunks from review comments.

use regex::Regex;
use std::fmt::Write;
use std::sync::LazyLock;

use crate::ReviewComment;

/// Width of the line number gutter in diff output
pub const GUTTER_WIDTH: usize = 5;

static HUNK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"@@ -(?P<old>\d+)(?:,(?P<old_count>\d+))? \+(?P<new>\d+)(?:,(?P<new_count>\d+))? @@",
    )
    .expect("valid regex")
});

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
    let diff_lines: Vec<&str> = comment
        .diff_hunk
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .collect();
    let mut lines_iter = diff_lines.iter().copied();
    let Some(header) = lines_iter.next() else {
        return Ok(String::new());
    };

    let lines: Vec<(Option<i32>, Option<i32>, String)> = HUNK_RE.captures(header).map_or_else(
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

    #[test]
    fn hunk_re_variants() {
        let caps = HUNK_RE.captures("@@ -1 +2 @@").expect("regex");
        assert_eq!(&caps["old"], "1");
        assert!(caps.name("old_count").is_none());
        assert_eq!(&caps["new"], "2");
        assert!(caps.name("new_count").is_none());

        let caps = HUNK_RE.captures("@@ -3,4 +5 @@").expect("regex");
        assert_eq!(&caps["old"], "3");
        assert_eq!(caps.name("old_count").expect("old count").as_str(), "4");
        assert_eq!(&caps["new"], "5");
        assert!(caps.name("new_count").is_none());

        let caps = HUNK_RE.captures("@@ -7 +8,2 @@").expect("regex");
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
