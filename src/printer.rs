//! Helpers for formatting and printing review comments.
#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "internal helpers"
)]

use crate::html::collapse_details;
use crate::ref_utils::{ReviewComment, ReviewThread};
use regex::Regex;
use std::sync::LazyLock;
use termimad::MadSkin;

/// Width of the line number gutter in diff output.
const GUTTER_WIDTH: usize = 5;

pub static HUNK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"@@ -(?P<old>\d+)(?:,(?P<old_count>\d+))? \+(?P<new>\d+)(?:,(?P<new_count>\d+))? @@",
    )
    .expect("valid regex")
});

fn num_disp(num: i32) -> String {
    let mut s = num.to_string();
    if s.len() > GUTTER_WIDTH {
        let start = s.len() - GUTTER_WIDTH;
        s = s.split_off(start);
    }
    format!("{s:>GUTTER_WIDTH$}")
}

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

/// Format a comment diff hunk for display.
pub fn format_comment_diff(comment: &ReviewComment) -> Result<String, std::fmt::Error> {
    use std::fmt::Write;

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

/// Write the body of a single review comment.
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

fn write_comment<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    comment: &ReviewComment,
) -> anyhow::Result<()> {
    let diff = format_comment_diff(comment)?;
    write!(out, "{diff}")?;
    write_comment_body(&mut out, skin, comment)?;
    Ok(())
}

/// Write a full review thread, showing the diff only once.
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

/// Print a review thread to stdout.
pub fn print_thread(skin: &MadSkin, thread: &ReviewThread) -> anyhow::Result<()> {
    write_thread(std::io::stdout().lock(), skin, thread)
}

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
