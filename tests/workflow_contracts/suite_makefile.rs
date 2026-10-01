//! Reading a Makefile for the suite-once contract: the goal a bare `make`
//! runs, and the recipe of one target.
//!
//! Kept apart from `suite_once.rs` so each file stays under the repository's
//! 400-line limit. Only that contract and its reader cases use it.

/// The feature whose tests the feature target runs.
pub(crate) const FEATURE: &str = "unstable-rest-resolve";

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

/// Which of the feature's two test groups a recipe line runs.
#[derive(PartialEq)]
enum Group {
    /// The `resolve` integration test.
    Integration,
    /// The `resolve::rest` unit tests.
    Unit,
}

/// The words of one recipe line.
struct Line<'a>(Vec<&'a str>);

impl<'a> Line<'a> {
    /// Splits a recipe line into words.
    fn new(line: &'a str) -> Self {
        Self(line.split_whitespace().collect())
    }

    /// Returns `true` if the line has `flag` as a word.
    fn has(&self, flag: &str) -> bool {
        self.0.contains(&flag)
    }

    /// Returns `true` if the line runs `$(CARGO) test`.
    fn runs_cargo_test(&self) -> bool {
        self.0.windows(2).any(|pair| pair == ["$(CARGO)", "test"])
    }

    /// Returns `true` if the line selects a scope other than one named target.
    fn selects_another_scope(&self) -> bool {
        ["--lib", "--doc", "--tests", "--bins"]
            .iter()
            .any(|flag| self.has(flag))
    }

    /// Returns the words after `$(CARGO) test`, less the trailing
    /// `$(BUILD_JOBS)` variable the recipe may carry.
    fn arguments(&self) -> Vec<&str> {
        let start = self
            .0
            .windows(2)
            .position(|pair| pair == ["$(CARGO)", "test"])
            .map_or(self.0.len(), |at| at + 2);
        self.0
            .iter()
            .skip(start)
            .copied()
            .filter(|word| *word != "$(BUILD_JOBS)")
            .collect()
    }

    /// Returns `true` if the line's arguments are exactly `expected`, so an
    /// extra filter, flag or target narrows nothing unnoticed.
    fn arguments_are(&self, expected: &[&str]) -> bool {
        self.arguments() == expected
    }

    /// Returns `true` if the line runs the `resolve` integration test alone,
    /// with no filter after it.
    fn is_integration_run(&self) -> bool {
        self.arguments_are(&["--features", FEATURE, "--test", "resolve"])
    }

    /// Returns `true` if the line runs the `resolve::rest` unit tests alone,
    /// with no further filter.
    fn is_unit_run(&self) -> bool {
        self.arguments_are(&["--features", FEATURE, "--bin", "$(APP)", "resolve::rest"])
    }

    /// Returns the group the line runs, or `None` for a line that is not a
    /// `$(CARGO) test` run of exactly one of them.
    fn group(&self) -> Option<Group> {
        if !self.runs_cargo_test() || self.selects_another_scope() {
            return None;
        }
        match (self.is_integration_run(), self.is_unit_run()) {
            (true, false) => Some(Group::Integration),
            (false, true) => Some(Group::Unit),
            _ => None,
        }
    }
}

impl Recipe {
    /// Returns every way the recipe strays from the feature's own tests: a
    /// line that is not a `cargo test` run of exactly one of the two groups,
    /// one that drops the feature, one that widens to every target or
    /// feature and so repeats the suite, or a missing group.
    pub(crate) fn faults(&self) -> Vec<String> {
        let flag = format!("--features {FEATURE}");
        let mut faults: Vec<String> = self
            .0
            .iter()
            .flat_map(|line| {
                let mut found = Vec::new();
                if !line.contains(&flag) {
                    found.push(format!("`{line}` does not enable {FEATURE}"));
                }
                for widening in ["--all-targets", "--all-features", "--workspace"] {
                    if line.contains(widening) {
                        found.push(format!("`{line}` widens the run with {widening}"));
                    }
                }
                if Line::new(line).group().is_none() {
                    found.push(format!(
                        "`{line}` is not a `$(CARGO) test` run of one of the feature's two test groups"
                    ));
                }
                found
            })
            .collect();
        let groups: Vec<Group> = self
            .0
            .iter()
            .filter_map(|line| Line::new(line).group())
            .collect();
        if !groups.contains(&Group::Integration) {
            faults.push("no line runs the `resolve` integration test".to_owned());
        }
        if !groups.contains(&Group::Unit) {
            faults.push("no line runs the `resolve::rest` unit tests".to_owned());
        }
        faults
    }
}
