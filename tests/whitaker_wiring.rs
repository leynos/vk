//! Regression tests for the Whitaker lint wiring in the Makefile and CI.
//!
//! The suite runs through `make lint`, so these tests read the Makefile and the
//! pull-request workflow as text and refuse a change that drops Whitaker from
//! the lint gate, weakens its invocation, or installs it after the step that
//! needs it. They check shape, not the approved revision's value: concordat's
//! QG-002 owns which `install-whitaker` revision is acceptable.

use cap_std::ambient_authority;
use cap_std::fs::Dir;

/// Read a repository file as text.
fn read(path: &str) -> Result<String, String> {
    let root = Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), ambient_authority())
        .map_err(|error| format!("open the repository root: {error}"))?;
    root.read_to_string(path)
        .map_err(|error| format!("read {path}: {error}"))
}

/// Return the text of a Make rule: its header line and its recipe lines.
fn rule<'a>(makefile: &'a str, target: &str) -> Result<Vec<&'a str>, String> {
    let header = format!("{target}:");
    let mut lines = makefile
        .lines()
        .skip_while(|line| !line.starts_with(&header));
    let first = lines
        .next()
        .ok_or_else(|| format!("the Makefile has no `{target}` rule"))?;
    let recipe = lines.take_while(|line| line.starts_with('\t') || line.is_empty());
    Ok(std::iter::once(first).chain(recipe).collect())
}

#[test]
fn lint_runs_both_the_clippy_and_the_whitaker_targets() -> Result<(), String> {
    let makefile = read("Makefile")?;
    let lint = rule(&makefile, "lint")?;
    let header = lint.first().copied().unwrap_or_default();

    assert!(
        header.contains("lint-clippy"),
        "lint must run lint-clippy: {header}"
    );
    assert!(
        header.contains("lint-whitaker"),
        "lint must run lint-whitaker: {header}"
    );
    Ok(())
}

#[test]
fn lint_whitaker_uses_the_wrapper_with_clean_inputs_and_warnings_denied() -> Result<(), String> {
    let makefile = read("Makefile")?;
    let recipe = rule(&makefile, "lint-whitaker")?.join("\n");

    assert!(
        recipe.contains("$(WHITAKER_CLEAN_ENV)"),
        "driver inputs not cleaned: {recipe}"
    );
    assert!(
        recipe.contains("DYLINT_RUSTFLAGS=\"-D warnings\""),
        "warnings not denied: {recipe}"
    );
    assert!(
        recipe.contains("$(WHITAKER) --all -- --all-targets --all-features"),
        "wrapper invocation or scope changed: {recipe}"
    );
    assert!(
        !recipe.contains("cargo dylint"),
        "bare cargo dylint: {recipe}"
    );
    // The assignment continues over backslash-ended lines; read all of them.
    let mut definition = String::new();
    let mut continues = false;
    for line in makefile.lines() {
        if line.starts_with("WHITAKER_CLEAN_ENV") || continues {
            definition.push_str(line);
            definition.push('\n');
            continues = line.ends_with('\\');
        }
    }
    assert!(!definition.is_empty(), "WHITAKER_CLEAN_ENV is not defined");
    for variable in [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_PROFILE_DEV_CODEGEN_BACKEND",
        "CARGO_BUILD_TARGET",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
        "CFLAGS",
        "LDFLAGS",
    ] {
        let unset = format!("-u {variable}");
        assert!(
            definition
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| { pair.join(" ") == unset }),
            "the clean environment no longer unsets {variable}: {definition}"
        );
    }
    Ok(())
}

#[test]
fn ci_installs_whitaker_before_the_main_lint_step() -> Result<(), String> {
    let workflow = read(".github/workflows/coverage.yml")?;
    let (before, after) = workflow
        .split_once("leynos/shared-actions/.github/actions/install-whitaker@")
        .ok_or("CI does not install Whitaker")?;
    let pin: String = after.chars().take(40).collect();
    let next = after.chars().nth(40);
    assert!(
        pin.len() == 40
            && pin.chars().all(|c| c.is_ascii_hexdigit())
            && next.is_none_or(|c| !c.is_ascii_hexdigit()),
        "install-whitaker is not pinned to a full commit SHA: {pin}"
    );
    let lint = workflow
        .find("run: make lint\n")
        .ok_or("CI has no `make lint` step")?;
    assert!(
        before.len() < lint,
        "Whitaker must be installed before `make lint`"
    );
    Ok(())
}

#[test]
fn the_feature_job_lints_with_clippy_alone() -> Result<(), String> {
    let workflow = read(".github/workflows/coverage.yml")?;
    let feature = workflow
        .lines()
        .find(|line| {
            line.contains("--features unstable-rest-resolve") && line.contains("make lint")
        })
        .ok_or("no step lints the feature")?;

    assert!(
        feature.contains("make lint-clippy"),
        "feature lint must be lint-clippy: {feature}"
    );
    assert!(
        !feature.contains("make lint CLIPPY_FLAGS"),
        "feature lint would repeat Whitaker: {feature}"
    );
    Ok(())
}
