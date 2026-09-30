//! Cases for the suite-once contract's reader: which command lines run the
//! suite, whatever wrapper, separator, quote or comment surrounds them, and
//! which default goal a Makefile gives a bare `make`.
//!
//! These drive `shell::runs_suite` and `Makefile::default_goal` with constructed
//! text, because a rule checked only against workflows that already conform
//! passes whether or not it discriminates. The default-goal cases are also
//! run against real GNU make.

use rstest::rstest;

use super::{makefile::Makefile, shell};

/// The default goal the spelling cases assume: one that runs the suite.
const SUITE_GOAL: &str = "all";

#[rstest]
#[case::plain("make test", true)]
#[case::act_flag("make test WITH_ACT=1", true)]
#[case::make_option("make -j2 test", true)]
#[case::make_directory("make -C . test", true)]
#[case::make_all("make all", true)]
#[case::make_default_goal("make", true)]
#[case::make_coverage("make coverage", true)]
#[case::make_dev_test("make dev-test", true)]
#[case::quoted_target("make \"test\"", true)]
#[case::cargo("cargo test --all-features", true)]
#[case::cargo_config("cargo --config tools/dev-fast/config.toml test", true)]
#[case::cargo_toolchain("cargo +nightly test", true)]
#[case::nextest("cargo nextest run --all-targets", true)]
#[case::llvm_cov("cargo llvm-cov nextest --lcov", true)]
#[case::chained("set -eu && make test", true)]
#[case::unspaced("make lint&&make test", true)]
#[case::assignment("RUSTFLAGS=-Dwarnings cargo test", true)]
#[case::named_target("make test-workflow-contracts", false)]
#[case::lint("make lint", false)]
#[case::build("cargo build --all-targets", false)]
#[case::run_argument("cargo run -- test", false)]
#[case::echo("echo cargo test", false)]
#[case::env_wrapper("env RUSTFLAGS=x cargo test", true)]
#[case::timeout_wrapper("timeout 30m make test", true)]
#[case::bash_script("bash -c \"cargo test\"", true)]
#[case::sh_script("sh -c 'make test'", true)]
#[case::compound("if true; then cargo test; fi", true)]
#[case::shell_file("bash scripts/check.sh", false)]
#[case::nohup_wrapper("nohup cargo test", true)]
#[case::sudo_wrapper("sudo -u ci make test", true)]
#[case::shell_option_cluster("bash -lc 'make test'", true)]
#[case::trailing_comment("make test # the suite", true)]
#[case::quoted_separator("echo \"done; cargo test\"", false)]
#[case::commented_command("make lint # then make test", false)]
#[case::script_operand_only("bash -c 'cargo' test", false)]
#[case::make_dry_run("make -n", false)]
#[case::make_clustered_dry_run("make -ns", false)]
#[case::make_help("make --help", false)]
#[case::make_jobs("make -j4", true)]
#[case::make_jobs_number("make -j 4", true)]
#[case::make_load_number("make -l 4", true)]
#[case::make_jobs_target("make -j test", true)]
#[case::make_jobs_harmless_target("make -j lint", false)]
#[case::make_jobs_other_target("make -j 4 lint", false)]
#[case::escaped_quote_hides_separator("echo \"a \\\" ; make test\"", false)]
#[case::escaped_quote_then_suite("echo \"a \\\" b\" ; make test", true)]
#[case::escaped_backslash_closes("echo \"a \\\\\" ; make test", true)]
#[case::unterminated_quote_runs_on("echo \"make test", false)]
#[case::unterminated_quote_suite("make \"test", true)]
#[case::cargo_alias("cargo t", true)]
#[case::subshell("(cd crate && cargo test)", true)]
#[case::command_substitution("echo $(make test)", true)]
#[case::continuation_inside_a_word("cargo te\\\nst", true)]
#[case::continuation_after_a_space("make \\\ntest", true)]
#[case::continuation_joins_words("make\\\ntest", false)]
#[case::empty("", false)]
#[case::blank("   ", false)]
#[case::lone_separator("&", false)]
#[case::quoted_assignment("RUSTFLAGS='-D warnings' cargo test", true)]
#[case::commented_separator("echo ok # ; make test", false)]
#[case::clustered_shell_options("sh -ec 'make test'", true)]
#[case::command_lookup("command -v make", false)]
fn suite_spellings_are_recognized(#[case] line: &str, #[case] expected: bool) {
    assert_eq!(
        shell::runs_suite(line, SUITE_GOAL),
        expected,
        "misread {line:?}"
    );
}

/// Commands that run the suite, for the bounded composition tests.
const SUITE_RUNS: [&str; 4] = ["make test", "make", "cargo test", "cargo nextest run"];

/// Commands that run nothing of the suite, for the same tests.
const HARMLESS: [&str; 3] = ["make lint", "echo ok", "cargo build"];

/// Every way one command can follow another on a `run:` body.
const JOINERS: [&str; 6] = [";", " ; ", "&&", " || ", " | ", "\n"];

/// Every prefix the reader must look through to the command behind it.
const PREFIXES: [&str; 6] = ["", "X=1 ", "env X=1 ", "timeout 5m ", "then ", "do "];

/// Returns each harmless command joined to each command behind each prefix.
fn compositions(commands: &[&str]) -> Vec<String> {
    HARMLESS
        .iter()
        .flat_map(|first| {
            JOINERS.iter().flat_map(move |joiner| {
                PREFIXES.iter().flat_map(move |prefix| {
                    commands
                        .iter()
                        .map(move |command| format!("{first}{joiner}{prefix}{command}"))
                })
            })
        })
        .collect()
}

/// Every suite run is found after any joiner and behind any prefix; the
/// inputs are enumerated exhaustively rather than sampled.
#[test]
fn a_suite_run_is_found_wherever_it_is_joined() {
    let missed: Vec<String> = compositions(&SUITE_RUNS)
        .into_iter()
        .filter(|line| !shell::runs_suite(line, SUITE_GOAL))
        .collect();
    assert!(missed.is_empty(), "suite runs missed: {missed:?}");
}

/// No harmless command reads as a suite run, however it is joined.
#[test]
fn nothing_is_found_in_harmless_commands() {
    let found: Vec<String> = compositions(&HARMLESS)
        .into_iter()
        .filter(|line| shell::runs_suite(line, SUITE_GOAL))
        .collect();
    assert!(
        found.is_empty(),
        "harmless commands read as suite runs: {found:?}"
    );
}

/// A bare `make` runs the suite only when the Makefile's default goal does.
#[rstest]
#[case::builds("build", false)]
#[case::runs_all("all", true)]
#[case::runs_test("test", true)]
fn a_bare_make_runs_the_default_goal(#[case] goal: &str, #[case] expected: bool) {
    assert_eq!(shell::runs_suite("make", goal), expected);
}

#[rstest]
#[case::first_rule(".PHONY: a\nbuild: x\nall: y\n", "build")]
#[case::assigned(".DEFAULT_GOAL := test\nbuild:\n", "test")]
#[case::conditional_assignment_changes_nothing(".DEFAULT_GOAL ?= test\nbuild:\n", "build")]
#[case::plain_assignment(".DEFAULT_GOAL = test\nbuild:\n", "test")]
#[case::last_assignment_wins(".DEFAULT_GOAL := first\n.DEFAULT_GOAL := second\nbuild:\n", "second")]
#[case::conditional_keeps_the_first(
    ".DEFAULT_GOAL := first\n.DEFAULT_GOAL ?= second\nbuild:\n",
    "first"
)]
#[case::conditional_then_set(".DEFAULT_GOAL ?= second\n.DEFAULT_GOAL := first\nbuild:\n", "first")]
#[case::a_later_change_to_test(".DEFAULT_GOAL := build\nlint:\n.DEFAULT_GOAL := test\n", "test")]
#[case::an_empty_value_clears_it(".DEFAULT_GOAL := first\n.DEFAULT_GOAL :=\nbuild:\n", "build")]
#[case::several_words_are_refused(
    ".DEFAULT_GOAL := first\n.DEFAULT_GOAL += second\nbuild:\n",
    "build"
)]
#[case::spaced_operator(".DEFAULT_GOAL   :=   spaced\nbuild:\n", "spaced")]
#[case::appending_to_nothing_sets_it(".DEFAULT_GOAL += test\nbuild:\n", "test")]
#[case::recipe_text_is_not_an_assignment("first:\n\t.DEFAULT_GOAL = test\n", "first")]
#[case::comments_are_skipped("# build: not a rule\nrun: z\n", "run")]
#[case::special_targets_are_skipped(".PHONY: a\n.SUFFIXES:\nrun: z\n", "run")]
#[case::skips_variables_and_patterns("X := 1\n\tfoo: bar\n%.o: %.c\nrun: z\n", "run")]
#[case::none("X := 1\n", "")]
fn the_default_goal_is_read(#[case] makefile: &str, #[case] expected: &str) {
    assert_eq!(Makefile(makefile).default_goal(), expected);
}

/// Every wrapper is looked through, with an option of its own, to the
/// command it runs.
#[rstest]
#[case::env("env -u HOME")]
#[case::timeout("timeout -s KILL 5m")]
#[case::nice("nice -n 5")]
#[case::command("command")]
#[case::exec("exec -a name")]
#[case::time("time")]
#[case::nohup("nohup")]
#[case::setsid("setsid")]
#[case::stdbuf("stdbuf -oL")]
#[case::sudo("sudo -u ci")]
fn every_wrapper_is_looked_through(#[case] prefix: &str) {
    let runs = |command: &str| shell::runs_suite(&format!("{prefix} {command}"), SUITE_GOAL);
    assert!(runs("make test"), "{prefix} hid a suite run");
    assert!(
        !runs("make lint"),
        "{prefix} made a harmless command a suite run"
    );
}

/// Every make target the reader counts as running the suite.
#[rstest]
#[case::make_test("test")]
#[case::make_all("all")]
#[case::make_coverage("coverage")]
#[case::make_dev_test("dev-test")]
#[case::make_test_fast("test-fast")]
fn every_suite_target_runs_the_suite(#[case] target: &str) {
    assert!(shell::runs_suite(&format!("make {target}"), "build"));
    assert!(shell::runs_suite(&format!("make -j 4 {target}"), "build"));
    assert!(!shell::runs_suite(&format!("make {target}-not"), "build"));
}

/// Every make option that reads or describes the makefile without running a
/// goal, alone and beside a suite target.
#[rstest]
#[case::just_print("--just-print")]
#[case::dry_run("--dry-run")]
#[case::recon("--recon")]
#[case::short_dry_run("-n")]
#[case::question("--question")]
#[case::short_question("-q")]
#[case::help("--help")]
#[case::version("--version")]
#[case::short_version("-v")]
#[case::short_help("-h")]
#[case::clustered("-ns")]
fn every_inert_make_option_runs_no_goal(#[case] option: &str) {
    assert!(!shell::runs_suite(&format!("make {option}"), "all"));
    assert!(!shell::runs_suite(&format!("make {option} test"), "all"));
    assert!(!shell::runs_suite(&format!("make test {option}"), "all"));
}

/// `command -v` and `command -V` describe a command and run nothing.
#[rstest]
#[case::lower("command -v make test")]
#[case::upper("command -V make test")]
#[case::upper_bare("command -V make")]
fn command_lookup_runs_nothing(#[case] line: &str) {
    assert!(!shell::runs_suite(line, "all"));
}

/// Returns what GNU make itself takes as the default goal of `makefile`, or
/// `None` when make refuses it. `make -pn` prints the variable database
/// without running a recipe, and `.DEFAULT_GOAL` is the value make settled on
/// after reading every assignment (GNU make manual, "Other Special Variables").
#[cfg(target_os = "linux")]
fn make_default_goal(makefile: &str) -> std::io::Result<Option<String>> {
    use std::{
        io::Write as _,
        process::{Command as Process, Stdio},
    };

    let mut child = Process::new("make")
        .args(["-f", "-", "-pn"])
        .env_remove("MAKEFLAGS")
        .env_remove("MAKELEVEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("make's stdin was not piped"))?
        .write_all(makefile.as_bytes())?;
    let output = child.wait_with_output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(output.status.success().then_some(()).and_then(|()| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(" = ").or_else(|| line.split_once(" := "))?;
            (name == ".DEFAULT_GOAL").then(|| value.to_owned())
        })
    }))
}

/// The reader agrees with GNU make on every Makefile it can be run on, so the
/// behaviour is pinned to make and not to anyone's reading of its manual.
#[cfg(target_os = "linux")]
#[rstest]
#[case::first_rule(".PHONY: a\nbuild: x\nx:\n")]
#[case::assigned(".DEFAULT_GOAL := test\nbuild:\ntest:\n")]
#[case::conditional(".DEFAULT_GOAL ?= test\nbuild:\ntest:\n")]
#[case::plain(".DEFAULT_GOAL = test\nbuild:\ntest:\n")]
#[case::last_assignment_wins(
    ".DEFAULT_GOAL := first\n.DEFAULT_GOAL := second\nfirst:\nsecond:\nbuild:\n"
)]
#[case::conditional_keeps_the_first(
    ".DEFAULT_GOAL := first\n.DEFAULT_GOAL ?= second\nfirst:\nsecond:\n"
)]
#[case::conditional_then_set(".DEFAULT_GOAL ?= second\n.DEFAULT_GOAL := first\nfirst:\nsecond:\n")]
#[case::a_later_change_to_test(".DEFAULT_GOAL := build\nbuild:\ntest:\n.DEFAULT_GOAL := test\n")]
#[case::an_empty_value_clears_it(".DEFAULT_GOAL := first\n.DEFAULT_GOAL :=\nbuild:\nfirst:\n")]
#[case::appending_to_nothing(".DEFAULT_GOAL += test\nbuild:\ntest:\n")]
#[case::comments_are_skipped("# build: not a rule\nrun:\n")]
#[case::special_targets_are_skipped(".PHONY: a\n.SUFFIXES:\nrun:\n")]
#[case::recipe_text_is_not_an_assignment("first:\n\t@: .DEFAULT_GOAL = test\nsecond:\n")]
fn the_reader_agrees_with_gnu_make(#[case] makefile: &str) {
    let by_make = make_default_goal(makefile)
        .expect("make must run")
        .expect("make must accept the fixture");
    assert_eq!(Makefile(makefile).default_goal(), by_make, "{makefile:?}");
}

/// Make refuses a default goal of several targets; the reader does not read
/// such a value and falls back to the first rule.
#[cfg(target_os = "linux")]
#[test]
fn make_refuses_several_words_and_the_reader_does_not_read_them() {
    let makefile = ".DEFAULT_GOAL := first\n.DEFAULT_GOAL += second\nbuild:\nfirst:\nsecond:\n";
    assert_eq!(make_default_goal(makefile).expect("make must run"), None);
    assert_eq!(Makefile(makefile).default_goal(), "build");
}
