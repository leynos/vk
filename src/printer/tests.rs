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
    // Guard the banner icon
    assert!(out.contains("\u{1f4ac}"));
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
fn write_thread_prints_separator_after_each_comment_url() {
    let thread = ReviewThread {
        comments: CommentConnection {
            nodes: vec![
                ReviewComment {
                    body: "First".into(),
                    url: "https://example.com#discussion_r1".into(),
                    ..Default::default()
                },
                ReviewComment {
                    body: "Second".into(),
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
    let out = String::from_utf8(buf).expect("utf8");
    assert!(out.contains("https://example.com#discussion_r1\n---\n"));
    assert!(out.contains("https://example.com#discussion_r2\n---\n"));
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
        submitted_at: Some(Utc::now()),
        state: "APPROVED".into(),
        author: None,
    };
    let skin = MadSkin::default();
    let err = print_reviews(FailWriter, &skin, &[review]).expect_err("should fail");
    assert!(err.downcast_ref::<std::io::Error>().is_some());
}
