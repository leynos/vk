//! HTML utilities for comment rendering.

use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink as _;
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use std::default::Default;

/// Collapse root `<details>` blocks in the given text.
///
/// Each root-level `<details>` tag is replaced by the contents of its
/// direct `<summary>` child prefixed with a triangle marker. Nested
/// `<details>` blocks are discarded.
///
/// # Examples
///
/// ```
/// use vk::html::collapse_details;
/// let input = "<details><summary>hi</summary><p>hidden</p></details>";
/// assert_eq!(collapse_details(input), "\u25B6 hi\n");
/// ```
pub fn collapse_details(input: &str) -> String {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(input);
    let mut out = String::new();
    for child in dom.document.children.borrow().iter() {
        collapse_node(child, &mut out, false);
    }
    out
}

fn collapse_node(node: &Handle, out: &mut String, in_details: bool) {
    match &node.data {
        NodeData::Element { name, .. } if name.local.eq_str_ignore_ascii_case("details") => {
            if should_collapse_details(node, in_details) {
                write_collapsed_summary(node, out);
            }
            // drop children entirely when collapsing
        }
        NodeData::Element { .. } => {
            for child in node.children.borrow().iter() {
                collapse_node(child, out, in_details);
            }
        }
        NodeData::Text { contents } => {
            if !in_details {
                out.push_str(&contents.borrow());
            }
        }
        _ => {}
    }
}

fn should_collapse_details(node: &Handle, in_details: bool) -> bool {
    !in_details && find_summary_text(node).is_some()
}

fn write_collapsed_summary(node: &Handle, out: &mut String) {
    if let Some(summary) = find_summary_text(node) {
        out.push('\u{25B6}');
        out.push(' ');
        out.push_str(&summary);
        out.push('\n');
    }
}

fn find_summary_text(node: &Handle) -> Option<String> {
    for child in node.children.borrow().iter() {
        if let NodeData::Element { name, .. } = &child.data
            && name.local.eq_str_ignore_ascii_case("summary")
        {
            return Some(collect_text(child));
        }
    }
    None
}

fn collect_text(node: &Handle) -> String {
    let mut text = String::new();
    let mut stack = vec![node.clone()];
    while let Some(current) = stack.pop() {
        let children = current.children.borrow();
        for child in children.iter().rev() {
            match &child.data {
                NodeData::Text { contents } => text.push_str(&contents.borrow()),
                _ => stack.push(child.clone()),
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_replaces_root_details() {
        let input = concat!(
            "before\n",
            "<details><summary>sum</summary>hidden</details>\n",
            "after"
        );
        assert_eq!(collapse_details(input), "before\n\u{25B6} sum\n\nafter");
    }

    #[test]
    fn nested_details_are_hidden() {
        let input = "<details><summary>top</summary><details><summary>inner</summary>foo</details></details>";
        assert_eq!(collapse_details(input), "\u{25B6} top\n");
    }

    #[test]
    fn details_without_summary_removed() {
        let input = "<details><p>foo</p></details>";
        assert_eq!(collapse_details(input), "");
    }

    #[test]
    fn empty_details_block() {
        assert_eq!(collapse_details("<details></details>"), "");
    }

    #[test]
    fn malformed_html_is_handled() {
        let out = collapse_details("<details><summary>bad");
        assert!(out.contains("\u{25B6} bad"));
    }

    #[test]
    fn multiple_root_details() {
        let input = concat!(
            "<details><summary>one</summary>a</details>",
            "<details><summary>two</summary>b</details>"
        );
        assert_eq!(collapse_details(input), "\u{25B6} one\n\u{25B6} two\n");
    }
}
