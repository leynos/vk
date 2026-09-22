//! Properties of the reader and the publisher rule over generated workflows.
//!
//! The generated inputs are workflow-shaped: triggers written in each form
//! GitHub accepts, branch filters written as a scalar or a sequence, and
//! conditions built from the conjuncts the upload guard actually uses. A
//! free-form document would almost never put a value where the reader looks,
//! and a property over it would stay green with the reader broken.

use std::collections::BTreeSet;

use proptest::prelude::*;

use crate::codescene::is_publisher;
use crate::reader::{conjuncts, parse_workflow, references_secret};

/// How a trigger block is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// `on: [push, pull_request]`, which carries no filters.
    List,
    /// `on:` with one key per event, which may carry filters.
    Mapping,
}

/// How a single-branch filter is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spelling {
    /// `branches: main`.
    Scalar,
    /// `branches: [main]`.
    Sequence,
}

/// One generated trigger block, and what it should mean.
#[derive(Debug, Clone)]
struct Triggers {
    /// The events the workflow answers, in the order they are written.
    events: Vec<&'static str>,
    /// The branches the push is filtered to.
    branches: BTreeSet<&'static str>,
    /// Whether the push is also filtered to tags, and so fires for them.
    has_tags: bool,
    /// How a single branch is written.
    spelling: Spelling,
    /// How the block is written.
    form: Form,
}

impl Triggers {
    /// Return the block as a workflow would spell it.
    fn render(&self) -> String {
        if self.form == Form::List {
            return format!("on: [{}]\n", self.events.join(", "));
        }
        self.events
            .iter()
            .fold(String::from("on:\n"), |text, event| {
                let filter = if *event == "push" {
                    self.push_filter()
                } else {
                    String::new()
                };
                format!("{text}  {event}:\n{filter}")
            })
    }

    /// Return the push trigger's filter lines.
    fn push_filter(&self) -> String {
        let names: Vec<&str> = self.branches.iter().copied().collect();
        let branches = match (names.as_slice(), self.spelling) {
            ([], _) => String::new(),
            ([one], Spelling::Scalar) => format!("    branches: {one}\n"),
            (many, _) => format!("    branches: [{}]\n", many.join(", ")),
        };
        let tags = if self.has_tags {
            "    tags: ['v*']\n"
        } else {
            ""
        };
        format!("{branches}{tags}")
    }

    /// Return whether the block describes a push to `main` and nothing else.
    fn is_expected_publisher(&self) -> bool {
        self.events.contains(&"push")
            && !self.events.contains(&"pull_request")
            && self.form == Form::Mapping
            && self.branches == BTreeSet::from(["main"])
            && !self.has_tags
    }
}

/// Generate trigger blocks answering at least one event.
fn triggers() -> impl Strategy<Value = Triggers> {
    (
        proptest::sample::subsequence(vec!["push", "pull_request", "workflow_dispatch"], 1..=3),
        proptest::sample::subsequence(vec!["main", "release", "dev"], 0..=3),
        any::<bool>(),
        prop_oneof![Just(Spelling::Scalar), Just(Spelling::Sequence)],
        prop_oneof![Just(Form::List), Just(Form::Mapping)],
    )
        .prop_map(|(events, branches, has_tags, spelling, form)| Triggers {
            events,
            branches: branches.into_iter().collect(),
            has_tags,
            spelling,
            form,
        })
}

/// The conjuncts an upload guard is built from.
const ATOMS: [&str; 4] = [
    "env.CS_ACCESS_TOKEN != ''",
    "github.ref == 'refs/heads/main'",
    "github.event_name == 'push'",
    "matrix.os == 'a || b'",
];

proptest! {
    #[test]
    fn the_publisher_is_recognized_in_every_trigger_form(block in triggers()) {
        let workflow = parse_workflow("x.yml", &format!("{}jobs: {{}}\n", block.render()))
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert_eq!(is_publisher(&workflow), block.is_expected_publisher(), "{}", block.render());
    }

    #[test]
    fn conditions_split_into_their_conjuncts(
        picks in proptest::sample::subsequence(ATOMS.to_vec(), 1..=4),
        padding in "[ ]{0,3}",
    ) {
        let condition = picks.join(&format!("{padding}&&{padding}"));
        let expected: Vec<String> = picks.iter().map(|atom| (*atom).to_owned()).collect();
        prop_assert_eq!(conjuncts(&condition), Ok(expected));
        let widened = format!("{condition} || github.event_name == 'workflow_dispatch'");
        prop_assert!(conjuncts(&widened).is_err());
    }

    #[test]
    fn the_token_is_recognized_in_any_letter_case(
        name in "[cC][sS]_[aA][cC][cC][eE][sS][sS]_[tT][oO][kK][eE][nN]",
        spelling in 0_usize..3,
        suffix in "[A-Za-z0-9_]",
    ) {
        let reference = match spelling {
            0 => format!("secrets.{name}"),
            1 => format!("secrets['{name}']"),
            _ => format!("secrets[\"{name}\"]"),
        };
        let expression = format!("${{{{ {reference} }}}}");
        prop_assert!(references_secret(&expression, "CS_ACCESS_TOKEN"), "{}", expression);
        let longer = format!("${{{{ secrets.{name}{suffix} }}}}");
        prop_assert!(!references_secret(&longer, "CS_ACCESS_TOKEN"), "{}", longer);
    }
}
