//! Reading a Makefile for the suite-once contract: the goal a bare `make`
//! runs, and the recipe of one target.
//!
//! Kept apart from `suite_once.rs` so each file stays under the repository's
//! 400-line limit. Only that contract and its reader cases use it.

/// A Makefile's text.
#[derive(Clone, Copy)]
pub(crate) struct Makefile<'a>(pub(crate) &'a str);

/// The goal a bare `make` runs.
#[derive(Clone, Copy)]
pub(crate) struct DefaultGoal<'a>(pub(crate) &'a str);

/// The lines of one target's recipe, trimmed.
pub(crate) struct Recipe(pub(crate) Vec<String>);

impl Makefile<'_> {
    /// Returns the goal a bare `make` runs.
    pub(crate) fn default_goal(self) -> String {
        default_goal_of(self.0)
    }

    /// Returns the recipe lines of one target, trimmed, up to the next rule.
    pub(crate) fn recipe(self, target: &str) -> Recipe {
        Recipe(
            self.0
                .lines()
                .skip_while(|line| !line.starts_with(&format!("{target}:")))
                .skip(1)
                .take_while(|line| line.starts_with('\t') || line.trim().is_empty())
                .filter(|line| line.starts_with('\t'))
                .map(|line| line.trim().to_owned())
                .collect(),
        )
    }
}

/// Reads the default goal from Makefile text: what `.DEFAULT_GOAL` holds once
/// every assignment has been applied in order (GNU make manual, "Other
/// Special Variables"), otherwise the first rule that
/// is not a special or pattern target.
fn default_goal_of(makefile: &str) -> String {
    let assigned = makefile
        .lines()
        .filter(|line| !line.starts_with('\t'))
        .filter_map(assignment_of)
        .fold(None, apply)
        .filter(|goal| !goal.contains(char::is_whitespace));
    assigned
        .or_else(|| makefile.lines().find_map(first_goal))
        .unwrap_or_default()
}

/// How an assignment to `.DEFAULT_GOAL` combines with the value before it.
#[derive(Clone, Copy)]
enum Assignment<'a> {
    /// `=` and `:=` replace the value, and an empty one clears it.
    Set(&'a str),
    /// `+=` appends a word, and make refuses a default goal of several.
    Append(&'a str),
}

/// Returns the assignment to `.DEFAULT_GOAL` a line makes, if it makes one.
///
/// A `?=` line is not one: make defines `.DEFAULT_GOAL` itself, empty, before
/// it reads a makefile, so `?=` finds it defined and changes nothing.
fn assignment_of(line: &str) -> Option<Assignment<'_>> {
    let (left, right) = line.split_once(":=").or_else(|| line.split_once('='))?;
    let (name, value) = (left.trim_end(), right.trim());
    let is_goal = |base: &str| base.trim_end() == ".DEFAULT_GOAL";
    if name.strip_suffix('+').is_some_and(is_goal) {
        Some(Assignment::Append(value))
    } else {
        is_goal(name.trim_end_matches(':')).then_some(Assignment::Set(value))
    }
}

/// Returns the value of `.DEFAULT_GOAL` after one more assignment.
fn apply(current: Option<String>, assignment: Assignment<'_>) -> Option<String> {
    let value = match assignment {
        Assignment::Set(value) => Some(value.to_owned()),
        Assignment::Append(value) => Some(format!("{} {value}", current.unwrap_or_default())),
    };
    value
        .map(|goal| goal.trim().to_owned())
        .filter(|goal| !goal.is_empty())
}

/// Returns the first goal a rule line names, or `None` for any other line.
fn first_goal(line: &str) -> Option<String> {
    if line.starts_with(['\t', ' ', '#', '.']) {
        return None;
    }
    let (names, rest) = line.split_once(':')?;
    if rest.starts_with('=') || names.contains(['=', '%', '$']) {
        return None;
    }
    names.split_whitespace().next().map(str::to_owned)
}
