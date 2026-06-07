//! Hermetic temporary Git repository builder for tests.
//!
//! Exposes [`GitRepoFixture`], a `TempDir`-backed Git repository that the
//! `ref_parser` and `commands` test suites use to exercise `git`-aware code
//! paths without touching the user's real repository. All constructors and
//! builders return [`io::Result`] so a broken test environment (missing
//! `git` binary, disk full, …) surfaces a clear error rather than a panic.

use std::io;
use std::path::Path;

/// Run `git` with `args` inside `dir`, surfacing both spawn and non-zero
/// exit failures as `io::Error` so callers can propagate with `?`.
fn run_git_in(dir: &Path, args: &[&str]) -> io::Result<()> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "git {args:?} in {dir:?} failed (status {status}): {stderr}",
            status = output.status,
            stderr = stderr.trim(),
        )));
    }
    Ok(())
}

/// A temporary Git repository directory for tests.
///
/// Initialises a hermetic repo inside a [`tempfile::TempDir`] and exposes
/// builders for the shapes the `ref_parser` and `commands` test suites need:
/// a branch-pointing HEAD ([`Self::on_branch`]), a detached HEAD over an empty
/// commit ([`Self::detached`]), `FETCH_HEAD` contents ([`Self::with_fetch_head`]),
/// and an `origin` remote ([`Self::with_origin`]). The temporary directory is
/// removed when the fixture is dropped.
///
/// All constructors and builders return [`io::Result`] so a broken test
/// environment surfaces as a clear error rather than a panic deep inside a
/// helper; tests typically `.expect(...)` at the call site to keep the
/// failure mode unchanged.
///
/// The fixture intentionally does **not** change the process working
/// directory. Tests that exercise code paths reading from the current
/// directory (such as the public `repo_from_fetch_head` / `repo_from_origin`
/// helpers) should compose the fixture with [`super::CwdGuard`] and mark
/// themselves `#[serial]`.
pub struct GitRepoFixture {
    dir: tempfile::TempDir,
}

impl GitRepoFixture {
    /// Create a fixture whose HEAD is a symbolic ref to `branch`.
    ///
    /// No commit is required — `git symbolic-ref` is sufficient and keeps the
    /// fixture cheap. The `-c init.defaultBranch=main` flag keeps behaviour
    /// stable on Git versions below 2.28.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` when the temporary directory cannot be created,
    /// when `git` cannot be spawned, or when either `git init` or
    /// `git symbolic-ref` exits with a non-zero status.
    pub fn on_branch(branch: &str) -> io::Result<Self> {
        let dir = tempfile::TempDir::new()?;
        run_git_in(dir.path(), &["-c", "init.defaultBranch=main", "init"])?;
        run_git_in(
            dir.path(),
            &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")],
        )?;
        Ok(Self { dir })
    }

    /// Create a fixture with a detached HEAD pointing at an empty commit.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` when any of the underlying `git` invocations
    /// (`init`, `config`, `commit`, `checkout --detach`) cannot be spawned
    /// or exits with a non-zero status.
    pub fn detached() -> io::Result<Self> {
        let dir = tempfile::TempDir::new()?;
        run_git_in(dir.path(), &["-c", "init.defaultBranch=main", "init"])?;
        for (key, value) in [("user.email", "test@test.com"), ("user.name", "Test")] {
            run_git_in(dir.path(), &["config", key, value])?;
        }
        run_git_in(
            dir.path(),
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        )?;
        run_git_in(dir.path(), &["checkout", "--detach"])?;
        Ok(Self { dir })
    }

    /// Configure an `origin` remote pointing at `url`.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` when `git remote add` cannot be spawned or
    /// exits with a non-zero status.
    pub fn with_origin(self, url: &str) -> io::Result<Self> {
        run_git_in(self.dir.path(), &["remote", "add", "origin", url])?;
        Ok(self)
    }

    /// Write `content` to the repository's `FETCH_HEAD` file.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` when the file cannot be created or written.
    pub fn with_fetch_head(self, content: &str) -> io::Result<Self> {
        let fetch_head = self.dir.path().join(".git").join("FETCH_HEAD");
        std::fs::write(fetch_head, content)?;
        Ok(self)
    }

    /// Path to the repository's working directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}
