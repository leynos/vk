//! Shell command reading for the suite-once contract: splits a command
//! line into segments and words the way the shell does, then decides
//! whether a segment runs the test suite.
//!
//! Kept apart from `suite_once.rs` so each file stays under the repository's
//! 400-line limit. Only the suite-once contract uses it. The text sits
//! behind small types (`Word`, `Words`, `ValueOptions`, `Wrapper`), so each
//! question is a method on what it reads. The tokenizer is in
//! `suite_tokenizer.rs`.

use super::tokenizer::{Tokenizer, Word};

/// Cargo options that take their value as the next word.
const CARGO_VALUES: ValueOptions = ValueOptions(&[
    "--config",
    "-Z",
    "-C",
    "--manifest-path",
    "--color",
    "--target-dir",
]);

/// Make options that take their value as the next word.
const MAKE_VALUES: ValueOptions = ValueOptions(&[
    "-C",
    "-f",
    "-I",
    "-o",
    "-W",
    "--directory",
    "--file",
    "--makefile",
]);

/// Make options that read or describe the makefile without running a goal.
const MAKE_INERT_OPTIONS: [&str; 8] = [
    "--just-print",
    "--dry-run",
    "--recon",
    "--question",
    "--help",
    "--version",
    "-v",
    "-h",
];

/// Short make options that also run no goal (`-n`, `-q`), as they may be
/// clustered with others, as in `-ns`.
const MAKE_INERT_LETTERS: [char; 2] = ['n', 'q'];

/// Short make options that take a value, which ends a cluster of letters.
const MAKE_VALUE_LETTERS: [char; 7] = ['C', 'f', 'I', 'o', 'W', 'j', 'l'];

/// Cargo subcommands that run the suite.
const SUITE_SUBCOMMANDS: [&str; 4] = ["test", "t", "nextest", "llvm-cov"];

/// Make targets that run the suite: `test`, `all` (which runs it),
/// `coverage` (under `cargo llvm-cov`) and the fast local variants.
const SUITE_TARGETS: [&str; 5] = ["test", "all", "coverage", "dev-test", "test-fast"];

/// Wrappers that run a command after their own options.
const WRAPPERS: [Wrapper; 10] = [
    Wrapper::new("env", &["-u", "--unset", "-C", "--chdir"], 0),
    Wrapper::new("timeout", &["-s", "--signal", "-k", "--kill-after"], 1),
    Wrapper::new("nice", &["-n", "--adjustment"], 0),
    Wrapper::new("command", &[], 0),
    Wrapper::new("exec", &["-a"], 0),
    Wrapper::new("time", &[], 0),
    Wrapper::new("nohup", &[], 0),
    Wrapper::new("setsid", &[], 0),
    Wrapper::new(
        "stdbuf",
        &["-i", "-o", "-e", "--input", "--output", "--error"],
        0,
    ),
    Wrapper::new(
        "sudo",
        &[
            "-u",
            "--user",
            "-g",
            "--group",
            "-h",
            "--host",
            "-p",
            "--prompt",
            "-C",
            "-D",
            "--chdir",
            "-R",
            "--chroot",
            "-T",
            "--command-timeout",
            "-r",
            "--role",
            "-t",
            "--type",
        ],
        0,
    ),
];

/// Reserved words that open or continue a compound command, which follows.
const CONTROL_WORDS: [&str; 9] = [
    "if", "then", "else", "elif", "do", "while", "until", "!", "{",
];

/// Shells whose `-c` operand is itself a command.
const SHELLS: [&str; 2] = ["sh", "bash"];

/// Returns `true` if any segment of `command` runs the suite.
///
/// `default_goal` is the goal a bare `make` runs, read from the Makefile: a
/// bare `make` runs the suite only when that goal does.
pub(crate) fn runs_suite(command: &str, default_goal: &str) -> bool {
    let goal = Goal(default_goal);
    Tokenizer::default()
        .read(command)
        .iter()
        .any(|segment| Words(segment.words()).runs_suite(goal))
}

/// Returns the targets the `make` commands within `command` name, looking
/// through wrappers, so a contract can ask whether a step runs a given target.
pub(crate) fn make_targets(command: &str) -> Vec<String> {
    Tokenizer::default()
        .read(command)
        .iter()
        .flat_map(|segment| match Words(segment.words()).invocation() {
            Some(Invocation::Program(program, arguments)) if program.program_name() == "make" => {
                arguments
                    .make_operands()
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            }
            _ => Vec::new(),
        })
        .collect()
}

/// The goal a bare `make` runs.
#[derive(Clone, Copy)]
struct Goal<'a>(&'a str);

/// A run of words, the first of which is a program.
#[derive(Clone, Copy)]
struct Words<'a>(&'a [Word]);

/// Options that take their value as the next word.
struct ValueOptions(&'static [&'static str]);

/// A program that runs the command after its own options and operands.
struct Wrapper {
    /// The program's name.
    name: &'static str,
    /// The options that take a value.
    values: ValueOptions,
    /// The operands read before the command.
    leading: usize,
}

/// What a segment finally runs.
enum Invocation<'a> {
    /// A program and its arguments.
    Program(&'a Word, Words<'a>),
    /// The script a shell's `-c` option runs.
    Script(&'a Word),
}

impl Word {
    /// Returns the word's text.
    fn text(&self) -> &str {
        &self.0
    }

    /// Returns the program's name without its directory.
    fn program_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// Returns `true` if the word names `expected`.
    fn is(&self, expected: &str) -> bool {
        self.0 == expected
    }

    /// Returns `true` if the word is one of `names`.
    fn is_one_of(&self, names: &[&str]) -> bool {
        names.contains(&self.text())
    }

    /// Returns `true` for an option or a toolchain selector.
    fn is_flag(&self) -> bool {
        self.0.starts_with(['-', '+'])
    }

    /// Returns `true` for a `NAME=value` assignment.
    fn is_assignment(&self) -> bool {
        self.0.contains('=') && !self.0.starts_with('-')
    }

    /// Returns `true` for a make option whose value is optional: read only
    /// when the next word is a number, so `make -j test` still builds `test`.
    fn takes_number(&self) -> bool {
        self.is_one_of(&["-j", "-l", "--jobs", "--load-average", "--max-load"])
    }

    /// Returns `true` if the word is a number.
    fn is_number(&self) -> bool {
        self.0.parse::<f64>().is_ok()
    }

    /// Returns `true` for a short shell option group that includes `c`.
    fn is_command_flag(&self) -> bool {
        self.0.starts_with('-') && !self.0.starts_with("--") && self.0.contains('c')
    }

    /// Returns `true` for a make option that reads the makefile without
    /// running a goal: `-n`, `-q`, `--help` and their kin, clustered or not.
    fn is_inert_make_option(&self) -> bool {
        if self.is_one_of(&MAKE_INERT_OPTIONS) {
            return true;
        }
        let Some(letters) = self
            .0
            .strip_prefix('-')
            .filter(|rest| !rest.starts_with('-'))
        else {
            return false;
        };
        letters
            .chars()
            .take_while(|letter| {
                letter.is_ascii_alphabetic() && !MAKE_VALUE_LETTERS.contains(letter)
            })
            .any(|letter| MAKE_INERT_LETTERS.contains(&letter))
    }
}

impl ValueOptions {
    /// Returns `true` if `word` is an option that takes the next word.
    fn take(&self, word: &Word) -> bool {
        word.is_one_of(self.0)
    }
}

impl Wrapper {
    /// Builds a wrapper description.
    const fn new(name: &'static str, values: &'static [&'static str], leading: usize) -> Self {
        Self {
            name,
            values: ValueOptions(values),
            leading,
        }
    }

    /// Returns the wrapper a word names, if it names one.
    fn named(word: &Word) -> Option<&'static Self> {
        WRAPPERS.iter().find(|wrapper| word.is(wrapper.name))
    }
}

impl<'a> Words<'a> {
    /// Returns the words from the first one that is not an assignment.
    fn without_assignments(self) -> Self {
        let start = self
            .0
            .iter()
            .take_while(|word| word.is_assignment())
            .count();
        Self(self.0.get(start..).unwrap_or_default())
    }

    /// Returns the words after the first, or `None` when there are none.
    fn split_first(self) -> Option<(&'a Word, Self)> {
        self.0
            .split_first()
            .map(|(first, rest)| (first, Self(rest)))
    }

    /// Returns the words from the first operand on.
    fn after_options(self, values: &ValueOptions) -> Option<Self> {
        let mut index = 0_usize;
        while let Some(word) = self
            .0
            .get(index)
            .filter(|word| word.text().starts_with('-'))
        {
            index = index.checked_add(if values.take(word) { 2 } else { 1 })?;
        }
        Some(Self(self.0.get(index..)?))
    }

    /// Returns the operands: the words less options, their values,
    /// toolchain selectors and assignments.
    fn operands(self, values: &ValueOptions) -> Vec<&'a str> {
        let mut found = Vec::new();
        let mut words = self.0.iter();
        while let Some(word) = words.next() {
            if values.take(word) {
                words.next();
            } else if !word.is_flag() && !word.is_assignment() {
                found.push(word.text());
            }
        }
        found
    }

    /// Returns make's operands: the words less options, their values and
    /// assignments. `-j 4` and `-l 4` take the number that follows them.
    fn make_operands(self) -> Vec<&'a str> {
        let mut found = Vec::new();
        let mut words = self.0.iter().peekable();
        while let Some(word) = words.next() {
            if MAKE_VALUES.take(word) {
                words.next();
            } else if word.takes_number() {
                words.next_if(|value| value.is_number());
            } else if !word.is_flag() && !word.is_assignment() {
                found.push(word.text());
            }
        }
        found
    }

    /// Returns what the words finally run, looking through wrappers and
    /// reserved words. A shell's `-c` operand is returned as the script, to
    /// be read as a command of its own.
    fn invocation(self) -> Option<Invocation<'a>> {
        let (program, arguments) = self.without_assignments().split_first()?;
        if program.program_name().is_empty() {
            return None;
        }
        if program.is_one_of(&CONTROL_WORDS) {
            return arguments.invocation();
        }
        if SHELLS.contains(&program.program_name()) {
            return Some(
                arguments
                    .script()
                    .map_or(Invocation::Program(program, arguments), Invocation::Script),
            );
        }
        Self::behind_wrapper(program, arguments).map_or(
            Some(Invocation::Program(program, arguments)),
            Self::invocation,
        )
    }

    /// Returns the single operand a shell's `-c` option runs. Later words are
    /// positional parameters of that script, not part of it.
    fn script(self) -> Option<&'a Word> {
        let option = self.0.iter().position(Word::is_command_flag)?;
        self.0.get(option.checked_add(1)?)
    }

    /// Returns the command a wrapper runs, or `None` if `program` wraps
    /// nothing. `command -v make` describes `make` and runs nothing.
    fn behind_wrapper(program: &Word, arguments: Self) -> Option<Self> {
        let wrapper = Wrapper::named(program)?;
        if program.is("command") && arguments.0.iter().any(|word| word.is_one_of(&["-v", "-V"])) {
            return None;
        }
        let operands = arguments.after_options(&wrapper.values)?;
        Some(Self(operands.0.get(wrapper.leading..)?))
    }

    /// Returns `true` if the words run the suite.
    fn runs_suite(self, goal: Goal<'_>) -> bool {
        match self.invocation() {
            Some(Invocation::Script(script)) => runs_suite(script.text(), goal.0),
            Some(Invocation::Program(program, arguments)) => match program.program_name() {
                "cargo" => arguments.cargo_runs_suite(),
                "make" => arguments.make_runs_suite(goal),
                _ => false,
            },
            None => false,
        }
    }

    /// Returns `true` if cargo's arguments name a suite subcommand.
    fn cargo_runs_suite(self) -> bool {
        self.operands(&CARGO_VALUES)
            .first()
            .is_some_and(|subcommand| SUITE_SUBCOMMANDS.contains(subcommand))
    }

    /// Returns `true` if make's arguments run a suite goal. A bare `make`
    /// runs the default goal, and an option such as `-n` stops make before
    /// it runs any goal.
    fn make_runs_suite(self, goal: Goal<'_>) -> bool {
        if self.0.iter().any(Word::is_inert_make_option) {
            return false;
        }
        let targets = self.make_operands();
        let goals = if targets.is_empty() {
            vec![goal.0]
        } else {
            targets
        };
        goals.iter().any(|target| SUITE_TARGETS.contains(target))
    }
}
