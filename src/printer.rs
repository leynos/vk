//! Output formatting helpers.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "docs omitted"
)]

use crate::html::collapse_details;
use crate::models::{ReviewComment, ReviewThread};
use std::fmt::Write as _;
use termimad::MadSkin;

/// Width of the line number gutter in diff output
const GUTTER_WIDTH: usize = 5;

pub fn format_comment_diff(comment: &ReviewComment) -> Result<String, std::fmt::Error> {
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

    let mut lines_iter = comment.diff_hunk.lines();
    let Some(header) = lines_iter.next() else {
        return Ok(String::new());
    };

    let hunk_re: regex::Regex = regex::Regex::new(
        r"@@ -(?P<old>\d+)(?:,(?P<old_count>\d+))? \+(?P<new>\d+)(?:,(?P<new_count>\d+))? @@",
    )
    .expect("valid regex");

    let lines: Vec<(Option<i32>, Option<i32>, String)> = hunk_re.captures(header).map_or_else(
        || parse_diff_lines(comment.diff_hunk.lines(), None, None),
        |caps| {
            let old_start: i32 = caps
                .name("old")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let new_start: i32 = caps
                .name("new")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
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
        let disp = n.or(*o).map_or_else(|| " ".repeat(GUTTER_WIDTH), num_disp);
        writeln!(&mut out, "{disp}|{text}")?;
    }
    Ok(out)
}

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

pub fn print_thread(skin: &MadSkin, thread: &ReviewThread) -> anyhow::Result<()> {
    write_thread(std::io::stdout().lock(), skin, thread)
}

#[must_use]
pub fn summarize_files(threads: &[ReviewThread]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in threads {
        for c in &t.comments.nodes {
            *counts.entry(c.path.clone()).or_default() += 1;
        }
    }
    counts.into_iter().collect()
}

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

pub fn print_summary(summary: &[(String, usize)]) {
    let _ = write_summary(std::io::stdout().lock(), summary);
}

pub fn print_end_banner() {
    println!("========== end of code review ==========");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CommentConnection;
    use crate::reviews::PullRequestReview;
    use chrono::Utc;
    use rstest::*;

    #[fixture]
    fn review_comment(#[default("test.rs")] path: &str) -> ReviewComment {
        ReviewComment {
            path: path.into(),
            ..Default::default()
        }
    }

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
                    review_comment("a.rs"),
                    review_comment("b.rs"),
                ],
                ..Default::default()
            },
            ..Default::default()
        }],
        vec![("a.rs".into(), 2), ("b.rs".into(), 1)]
    )]
    fn summarize_files_counts_comments(
        #[case] threads: Vec<ReviewThread>,
        #[case] expected: Vec<(String, usize)>,
    ) {
        let summary = summarize_files(&threads);
        assert_eq!(summary, expected);
    }

    #[test]
    fn write_summary_outputs_text() {
        let summary = vec![("a.rs".into(), 2), ("b.rs".into(), 1)];
        let mut buf = Vec::new();
        write_summary(&mut buf, &summary).expect("write summary");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("Summary:"));
        assert!(out.contains("a.rs: 2 comments"));
        assert!(out.contains("b.rs: 1 comment"));
    }

    #[test]
    fn write_summary_handles_empty() {
        let summary: Vec<(String, usize)> = Vec::new();
        let mut buf = Vec::new();
        write_summary(&mut buf, &summary).expect("write summary");
        assert!(buf.is_empty());
    }

    #[test]
    fn write_thread_emits_diff_once() {
        let diff = "@@ -1 +1 @@\n-old\n+new\n";
        let c1 = ReviewComment {
            diff_hunk: diff.into(),
            url: "http://u1".into(),
            ..Default::default()
        };
        let c2 = ReviewComment {
            diff_hunk: diff.into(),
            url: "http://u2".into(),
            ..Default::default()
        };
        let thread = ReviewThread {
            comments: CommentConnection {
                nodes: vec![c1, c2],
                ..Default::default()
            },
            ..Default::default()
        };
        let skin = MadSkin::default();
        let mut buf = Vec::new();
        write_thread(&mut buf, &skin, &thread).expect("write thread");
        let out = String::from_utf8(buf).expect("utf8");
        assert_eq!(out.matches("|-old").count(), 1);
        assert_eq!(out.matches("wrote:").count(), 2);
    }

    #[test]
    fn comment_body_collapses_details() {
        let comment = ReviewComment {
            body: "<details><summary>note</summary>hidden</details>".into(),
            ..Default::default()
        };
        let skin = MadSkin::default();
        let mut buf = Vec::new();
        write_comment_body(&mut buf, &skin, &comment).expect("write comment");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("\u{25B6} note"));
        assert!(!out.contains("hidden"));
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

    #[test]
    fn review_body_collapses_details() {
        let review = PullRequestReview {
            body: "<details><summary>hello</summary>bye</details>".into(),
            submitted_at: Utc::now(),
            state: "APPROVED".into(),
            author: None,
        };
        let skin = MadSkin::default();
        let mut buf = Vec::new();
        crate::reviews::write_review(&mut buf, &skin, &review).expect("write review");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("\u{25B6} hello"));
        assert!(!out.contains("bye"));
    }
}
