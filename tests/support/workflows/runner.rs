//! How a job chooses its runner.
//!
//! Every form GitHub accepts for `runs-on` is modelled: a literal label, an
//! expression choosing between labels, a sequence of labels the runner must
//! all carry, and a `group`/`labels` mapping. A reader that returned "no
//! runner" for anything but a string made the sequence and mapping forms
//! invisible, so a job written `runs-on: [self-hosted, some-paid-label]` could
//! name a paid or unregistered runner while every contract skipped it.
//! Anything else is refused rather than read as a job without a runner.

use serde_norway::Value;

/// How a job chooses its runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunnerSelection {
    /// A single literal label, such as `ubuntu-latest`.
    Literal(String),
    /// An expression, kept with the arms it can evaluate to.
    Expression {
        /// The declaration exactly as the document carried it.
        raw: String,
        /// Every label the expression can evaluate to.
        arms: Vec<String>,
    },
    /// A sequence of labels, every one of which the runner must carry.
    Labels(Vec<String>),
    /// A `group`/`labels` mapping, which selects within a runner group.
    Group {
        /// The runner group named, when the mapping names one.
        group: Option<String>,
        /// The labels required within that group.
        labels: Vec<String>,
    },
    /// No `runs-on` at all: the job calls a reusable workflow.
    Delegated,
}

impl RunnerSelection {
    /// Return every label this selection can put a job on.
    pub(crate) fn labels(&self) -> Vec<String> {
        match self {
            Self::Literal(label) => vec![label.clone()],
            Self::Expression { arms, .. } => arms.clone(),
            Self::Labels(labels) | Self::Group { labels, .. } => labels.clone(),
            Self::Delegated => Vec::new(),
        }
    }

    /// Return whether this selection names a runner at all.
    pub(crate) const fn names_a_runner(&self) -> bool {
        !matches!(self, Self::Delegated)
    }

    /// Return the declaration as written, for the line-break check.
    ///
    /// The sequence and mapping forms are rendered rather than quoted, because
    /// a break inside one of their scalars is the same defect as a break in a
    /// bare one and has to stay visible to the reader that looks for it.
    pub(crate) fn raw(&self) -> String {
        match self {
            Self::Literal(label) => label.clone(),
            Self::Expression { raw, .. } => raw.clone(),
            Self::Labels(labels) => labels.join(", "),
            Self::Group { group, labels } => {
                format!("group {group:?} labels {}", labels.join(", "))
            }
            Self::Delegated => String::new(),
        }
    }
}

/// Return the expression arms a `runs-on` can evaluate to.
///
/// The arms are the single-quoted literals in the expression. Reading them
/// rather than matching the whole expression against a pattern means a lane
/// that is correctly placed but merely wrapped differently still passes, while
/// a lane whose fallback names the wrong label does not.
///
/// ```ignore
/// assert_eq!(arms_of("${{ fork && 'a' || 'b' }}"), vec!["a", "b"]);
/// assert_eq!(arms_of("'a' || 'b"), vec!["a"]);
/// ```
pub(crate) fn arms_of(raw: &str) -> Vec<String> {
    // Splitting on the quote leaves the quoted runs at the odd positions,
    // which avoids slicing the string: `indexing_slicing` and `string_slice`
    // are both denied here, and clippy lints tests under `--all-targets`.
    //
    // Only complete runs count. An expression with an odd number of quotes
    // has a dangling one, and the text after it sits at an odd position like
    // any closed run, so a reader taking every odd position would report the
    // remainder as an arm. `'ubuntu-latest' || 'ubicloud-standard-2` would
    // then present two arms and satisfy the fork-fallback contract while
    // GitHub evaluated something else entirely.
    //
    // Taking the segments in pairs is what tells the two apart. After the
    // leading segment, each closed run is followed by the text up to the next
    // quote, so a run with nothing following it was never closed.
    let segments: Vec<&str> = raw.split('\'').collect();
    segments
        .get(1..)
        .unwrap_or_default()
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .filter_map(|pair| pair.first())
        .map(|arm| (*arm).to_owned())
        .collect()
}

/// Return the strings of a sequence, in order.
fn strings_of(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Return a `runs-on` mapping's labels, whether written as one or as many.
fn labels_field_of(value: &Value) -> Vec<String> {
    match value.get("labels") {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Sequence(items)) => strings_of(items),
        _ => Vec::new(),
    }
}

/// Return the `runs-on` of one job, or `None` when it has no shape GitHub
/// accepts.
///
/// The caller turns `None` into an error naming the job, rather than reading
/// it as a job without a runner, which is how the sequence and mapping forms
/// once went unseen.
///
/// ```ignore
/// let job: Value = serde_norway::from_str("runs-on: [self-hosted, paid]")?;
/// assert_eq!(selection_of(&job), Some(RunnerSelection::Labels(vec!["self-hosted".into(), "paid".into()])));
/// ```
pub(crate) fn selection_of(job: &Value) -> Option<RunnerSelection> {
    let Some(value) = job.get("runs-on") else {
        return Some(RunnerSelection::Delegated);
    };
    match value {
        Value::String(raw) if raw.contains("${{") => Some(RunnerSelection::Expression {
            raw: raw.clone(),
            arms: arms_of(raw),
        }),
        Value::String(raw) => Some(RunnerSelection::Literal(raw.clone())),
        Value::Sequence(items) => Some(RunnerSelection::Labels(strings_of(items))),
        Value::Mapping(_) => Some(RunnerSelection::Group {
            group: value
                .get("group")
                .and_then(Value::as_str)
                .map(str::to_owned),
            labels: labels_field_of(value),
        }),
        _ => None,
    }
}
