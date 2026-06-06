//! Unit tests for printer helpers.

use super::*;

use chrono::Utc;
use rstest::rstest;

use crate::{
    CommentConnection, ReviewComment, ReviewThread, User,
    test_utils::{
        assert_diff_lines_not_blank_separated, assert_no_triple_newlines, strip_ansi_codes,
    },
};
use vk::icons::{ICON_COMMENT, ICON_FILE, ICON_PERMALINK};

const CODERABBIT_COMMENT: &str = include_str!("../../tests/fixtures/comment_coderabbit.txt");

#[test]
fn print_reviews_formats_authors_and_states() {
    let reviews = [
        PullRequestReview {
            body: "Needs work".into(),
            submitted_at: Some(Utc::now()),
            state: "CHANGES_REQUESTED".into(),
            author: Some(User {
                login: "alice".into(),
            }),
        },
        PullRequestReview {
            body: "Looks good".into(),
            submitted_at: Some(Utc::now()),
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
        submitted_at: Some(Utc::now()),
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
        submitted_at: Some(Utc::now()),
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
fn write_comment_body_formats_banner(#[case] login: Option<&str>, #[case] expected_login: &str) {
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
    // Ensure the banner icon is included in the rendered output.
    assert!(out.contains(ICON_COMMENT));
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

#[rstest]
#[case("", "")]
#[case("a", "a")]
#[case("a\nb", "a\nb")]
#[case("a\n\nb", "a\n\nb")]
#[case("a\n\n\nb", "a\n\nb")]
#[case("a\n\n\n\nb", "a\n\nb")]
#[case("a\n\n\nb\n\n\nc", "a\n\nb\n\nc")]
fn collapse_excessive_newlines_handles_edge_cases(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(collapse_excessive_newlines(input.to_string()), expected);
}

#[test]
fn write_comment_body_renders_coderabbit_comment() {
    let comment = ReviewComment {
        body: CODERABBIT_COMMENT.into(),
        ..Default::default()
    };
    let mut buf = Vec::new();
    write_comment_body(&mut buf, &MadSkin::default(), &comment).expect("write comment");
    let out = String::from_utf8(buf).expect("utf8");
    let plain = strip_ansi_codes(&out);
    assert_no_triple_newlines(&plain);
    assert!(
        plain.contains("▶ 📝 Committable suggestion"),
        "collapsed suggestion summary missing:\n{plain}"
    );
    assert_diff_lines_not_blank_separated(&plain, "printf");
}

#[test]
fn write_thread_emits_structured_layout_per_comment() {
    let thread = ReviewThread {
        comments: CommentConnection {
            nodes: vec![
                ReviewComment {
                    body: "First".into(),
                    diff_hunk: "@@ -1 +1 @@\n-old\n+new\n".into(),
                    path: "src/lib.rs".into(),
                    url: "https://example.com#discussion_r1".into(),
                    ..Default::default()
                },
                ReviewComment {
                    body: "Second".into(),
                    diff_hunk: "@@ -1 +1 @@\n-old\n+new\n".into(),
                    path: "src/lib.rs".into(),
                    url: "https://example.com#discussion_r2".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = Vec::new();
    write_thread(&mut buf, &MadSkin::default(), &thread).expect("write thread");
    let out = strip_ansi_codes(&String::from_utf8(buf).expect("utf8"));

    let url1_banner = format!("{ICON_PERMALINK} https://example.com#discussion_r1");
    let url2_banner = format!("{ICON_PERMALINK} https://example.com#discussion_r2");
    let path_banner = format!("{ICON_FILE} src/lib.rs:");

    // The first comment opens with a blank line followed by the URL.
    assert!(out.starts_with(&format!("\n{url1_banner}\n")));
    // Follow-up comments are preceded by the previous comment's closing
    // thematic break and a single blank line.
    assert!(out.contains(&format!("\n---\n\n{url2_banner}\n")));

    // The first comment renders the path and diff; the second omits both.
    assert!(out.contains(&format!("{path_banner}\n")));
    assert_eq!(out.matches(&path_banner).count(), 1);
    assert_eq!(out.matches("|-old").count(), 1);

    // Each comment block closes with `---` on its own line.
    assert_eq!(out.matches("\n---\n").count(), 2);

    // The URL precedes the body banner for both comments.
    let url1 = out.find(&url1_banner).expect("first URL");
    let url2 = out.find(&url2_banner).expect("second URL");
    let banner1 = out.find("First").expect("first body");
    let banner2 = out.find("Second").expect("second body");
    assert!(url1 < banner1, "first URL must precede first body");
    assert!(url2 < banner2, "second URL must precede second body");
}

#[test]
fn write_thread_frames_each_comment_with_single_blank_before_separator() {
    let thread = ReviewThread {
        comments: CommentConnection {
            nodes: vec![ReviewComment {
                body: "Only".into(),
                diff_hunk: "@@ -1 +1 @@\n-old\n+new\n".into(),
                path: "src/lib.rs".into(),
                url: "https://example.com#discussion_r1".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = Vec::new();
    write_thread(&mut buf, &MadSkin::default(), &thread).expect("write thread");
    let out = strip_ansi_codes(&String::from_utf8(buf).expect("utf8"));
    assert_no_triple_newlines(&out);
    // A single blank line precedes the closing thematic break.
    assert!(
        out.ends_with("\n\n---\n"),
        "output must end with one blank line before `---`: {out:?}"
    );
}

#[test]
fn write_thread_with_no_comments_produces_no_output() {
    let thread = ReviewThread {
        comments: CommentConnection {
            nodes: Vec::new(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = Vec::new();
    write_thread(&mut buf, &MadSkin::default(), &thread).expect("write thread");
    assert!(
        buf.is_empty(),
        "empty thread must produce no output: {buf:?}"
    );
}

#[test]
fn print_reviews_propagates_writer_errors() {
    let review = PullRequestReview {
        body: "Nice".into(),
        submitted_at: Some(Utc::now()),
        state: "APPROVED".into(),
        author: None,
    };
    let skin = MadSkin::default();
    let err = print_reviews(FailWriter, &skin, &[review]).expect_err("should fail");
    assert!(err.downcast_ref::<std::io::Error>().is_some());
}

#[test]
fn write_thread_propagates_writer_errors() {
    let thread = ReviewThread {
        comments: CommentConnection {
            nodes: vec![ReviewComment {
                body: "First".into(),
                url: "https://example.com#discussion_r1".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let skin = MadSkin::default();
    let err = write_thread(FailWriter, &skin, &thread).expect_err("should fail");
    assert!(err.downcast_ref::<std::io::Error>().is_some());
}

struct FailWriter;

impl std::io::Write for FailWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("fail"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
