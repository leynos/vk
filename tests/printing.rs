use rstest::*;
use std::fmt::Write;
use std::fs;
use termimad::MadSkin;
use vk::printer::{
    HUNK_RE, format_comment_diff, summarize_files, write_comment_body, write_summary, write_thread,
};
use vk::reviews::{PullRequestReview, write_review};
use vk::{CommentConnection, ReviewComment, ReviewThread};
#[test]
fn format_comment_diff_sample() {
    let data = fs::read_to_string("tests/fixtures/review_comment.json").expect("fixture");
    let comment: ReviewComment = serde_json::from_str(&data).expect("deserialize");
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
    vec![ReviewThread { comments: CommentConnection { nodes: vec![review_comment("a.rs"), review_comment("b.rs")], ..Default::default() }, ..Default::default() }],
    vec![("a.rs".into(), 1), ("b.rs".into(), 1)]
)]
#[case(
    vec![ReviewThread { comments: CommentConnection { nodes: vec![ review_comment("a.rs"), review_comment("a.rs"), review_comment("b.rs")], ..Default::default() }, ..Default::default() }],
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
fn review_body_collapses_details() {
    let review = PullRequestReview {
        body: "<details><summary>hello</summary>bye</details>".into(),
        submitted_at: chrono::Utc::now(),
        state: "APPROVED".into(),
        author: None,
    };
    let skin = MadSkin::default();
    let mut buf = Vec::new();
    write_review(&mut buf, &skin, &review).expect("write review");
    let out = String::from_utf8(buf).expect("utf8");
    assert!(out.contains("\u{25B6} hello"));
    assert!(!out.contains("bye"));
}
