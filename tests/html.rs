//! Tests for HTML utilities.

use vk::html::collapse_details;

#[test]
fn collapse_details_removes_carriage_returns() {
    let separator = format!("{CR}{LF}", CR = 0x000D as char, LF = '\n');
    let input = "line1\n```diff\n- a\n+ b\n```\n".replace('\n', &separator);
    let output = collapse_details(&input);
    assert!(!output.contains('\r'));
    assert!(output.contains("```diff\n- a\n+ b\n```"));
}
