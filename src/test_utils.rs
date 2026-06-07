//! Common test utilities for the `vk` binary crate.
//!
//! This module groups together a few unrelated test helpers so they can be
//! shared across both the binary's unit tests and its integration tests:
//!
//! - Environment-variable helpers and the [`EnvSandbox`] RAII guard, which
//!   acquires a global lock, snapshots the relevant configuration env vars and
//!   the current working directory, points discovery-related paths at an empty
//!   temporary tree, and restores everything on drop.
//! - The [`TestClient`] stub HTTP server and [`start_server`] helper used to
//!   exercise the GraphQL client without touching GitHub.
//! - [`GitRepoFixture`], a hermetic temporary Git repository builder used by
//!   tests that drive `git`-aware code (`current_branch`, `repo_from_origin`,
//!   `resolve_branch_and_repo`, …). Its `on_branch` / `detached` constructors
//!   spin up a real on-disk repo via `git init` and `symbolic-ref`; the
//!   `with_origin` / `with_fetch_head` builders configure remotes and
//!   `FETCH_HEAD` content. The repo lives in a `tempfile::TempDir` that is
//!   removed when the fixture is dropped.
//! - [`CwdGuard`], an RAII guard that switches the process working directory
//!   to a chosen path and restores it on drop. The guard acquires the same
//!   global lock as [`EnvSandbox`] via `try_lock`, so misuse (combining the
//!   two guards in a single test) fails fast with
//!   [`std::io::ErrorKind::WouldBlock`] rather than deadlocking. Tests should
//!   still mark themselves `#[serial]` to coordinate with other lock-aware
//!   guards.

use crate::api::{GraphQLClient, RetryConfig};
use vk::environment;

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use third_wheel::hyper::{
    Body, Request, Response, Server, StatusCode,
    service::{make_service_fn, service_fn},
};
use tokio::{task::JoinHandle, time::Duration};
pub use vk::test_utils::{
    apply_optional_env, assert_diff_lines_not_blank_separated, assert_no_triple_newlines,
    restore_optional_env, strip_ansi_codes,
};

/// Stub client and server handle for HTTP tests.
pub struct TestClient {
    /// Client targeting the stub server.
    pub client: GraphQLClient,
    /// Handle for stopping the server task.
    pub join: JoinHandle<()>,
    /// Count of HTTP requests received.
    pub hits: Arc<AtomicUsize>,
}

/// Start a stub HTTP server returning each body in `responses` sequentially.
///
/// Returns a [`GraphQLClient`] targeting the server and a [`JoinHandle`] for
/// the server task.
///
/// # Examples
///
/// ```no_run
/// use vk::test_utils::start_server;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let body = String::from("{}");
/// let server = start_server(vec![body]);
/// server.join.abort();
/// let _ = server.join.await;
/// # });
/// ```
pub fn start_server(responses: Vec<String>) -> TestClient {
    let responses = Arc::new(responses);
    let counter = Arc::new(AtomicUsize::new(0));
    let svc_counter = Arc::clone(&counter);
    let svc = make_service_fn(move |_conn| {
        let responses = Arc::clone(&responses);
        let counter = Arc::clone(&svc_counter);
        async move {
            Ok::<_, std::convert::Infallible>(service_fn(move |_req: Request<Body>| {
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                let body = responses
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| "{}".to_string());
                async move {
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Body::from(body))
                            .expect("response"),
                    )
                }
            }))
        }
    });
    let server = Server::bind(&"127.0.0.1:0".parse().expect("parse addr")).serve(svc);
    let addr = server.local_addr();
    let join = tokio::spawn(async move {
        let _ = server.await;
    });
    let retry = RetryConfig {
        base_delay: Duration::from_millis(1),
        jitter: false,
        ..RetryConfig::default()
    };
    let client = GraphQLClient::with_endpoint_retry("token", format!("http://{addr}"), None, retry)
        .expect("create client");
    TestClient {
        client,
        join,
        hits: counter,
    }
}

/// Guard that restores an environment variable to its original value on drop.
pub struct EnvGuard {
    key: &'static str,
    original: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        restore_env_key(self.key, self.original.take());
    }
}

/// Set `VK_HTTP_TIMEOUT` to an invalid value and restore it on drop.
#[must_use]
pub fn invalid_http_timeout_guard() -> EnvGuard {
    let guard = env_sandbox_lock();
    let original = environment::with_lock(|| {
        let original = env::var_os("VK_HTTP_TIMEOUT");
        // SAFETY: `EnvGuard` keeps `env_sandbox_lock()` held until drop.
        unsafe { env::set_var("VK_HTTP_TIMEOUT", "not-a-number") };
        original
    });
    EnvGuard {
        key: "VK_HTTP_TIMEOUT",
        original,
        _guard: guard,
    }
}

static ENV_SANDBOX_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const SANDBOXED_ENV_KEYS: &[&str] = &[
    "VK_REPO",
    "VK_GITHUB_TOKEN",
    "VK_TRANSCRIPT",
    "VK_HTTP_TIMEOUT",
    "VK_CONNECT_TIMEOUT",
    "VK_CONFIG_PATH",
    "CONFIG_PATH",
    "APPDATA",
    "LOCALAPPDATA",
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_CONFIG_DIRS",
];

fn env_sandbox_lock() -> MutexGuard<'static, ()> {
    ENV_SANDBOX_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("environment sandbox lock poisoned")
}

fn restore_env_key(key: &str, original: Option<OsString>) {
    environment::with_lock(|| match original {
        Some(value) => {
            // SAFETY: `environment::with_lock` serialises process-wide env access.
            unsafe { env::set_var(key, value) };
        }
        None => {
            // SAFETY: `environment::with_lock` serialises process-wide env access.
            unsafe { env::remove_var(key) };
        }
    });
}

/// RAII guard that isolates configuration discovery inputs for a test.
///
/// The sandbox snapshots relevant configuration environment variables and the
/// current working directory, points discovery-related paths at an empty
/// temporary directory, and restores everything on drop.
///
/// # Examples
///
/// ```ignore
/// let sandbox = vk::test_utils::EnvSandbox::new().expect("create sandbox");
/// let config_path = sandbox.path().join("vk.toml");
/// ```
pub struct EnvSandbox {
    current_dir: PathBuf,
    original_env: Vec<(&'static str, Option<OsString>)>,
    sandbox_dir: tempfile::TempDir,
    _guard: MutexGuard<'static, ()>,
}

impl EnvSandbox {
    /// Create a new isolated environment and working-directory sandbox.
    ///
    /// # Errors
    ///
    /// Returns an error when the temporary directory or current working
    /// directory cannot be created or switched.
    pub fn new() -> std::io::Result<Self> {
        let guard = env_sandbox_lock();
        let sandbox_dir = tempfile::tempdir()?;
        let sandbox_path = sandbox_dir.path().to_path_buf();
        let current_dir = environment::with_lock(env::current_dir)?;
        let original_env = environment::with_lock(|| {
            SANDBOXED_ENV_KEYS
                .iter()
                .map(|key| (*key, env::var_os(key)))
                .collect::<Vec<_>>()
        });

        env::set_current_dir(&sandbox_path)?;
        environment::with_lock(|| {
            for key in SANDBOXED_ENV_KEYS {
                // SAFETY: `environment::with_lock` serialises process-wide env access.
                unsafe { env::remove_var(key) };
            }
            for key in [
                "APPDATA",
                "LOCALAPPDATA",
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_CONFIG_DIRS",
            ] {
                // SAFETY: `environment::with_lock` serialises process-wide env access.
                unsafe { env::set_var(key, &sandbox_path) };
            }
        });

        Ok(Self {
            current_dir,
            original_env,
            sandbox_dir,
            _guard: guard,
        })
    }

    /// Return the root path of the temporary discovery sandbox.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.sandbox_dir.path()
    }
}

impl Drop for EnvSandbox {
    fn drop(&mut self) {
        for (key, value) in self.original_env.drain(..) {
            restore_env_key(key, value);
        }
        let _ = env::set_current_dir(&self.current_dir);
    }
}

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
/// helpers) should compose the fixture with [`CwdGuard`] and mark themselves
/// `#[serial]`.
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

/// RAII guard that switches the process working directory and restores it on
/// drop, holding the global env/sandbox lock for its lifetime.
///
/// The current working directory is process-global, so concurrent tests that
/// each call `set_current_dir` race. `CwdGuard::enter` shares the same mutex
/// as [`EnvSandbox`], ensuring serialised access regardless of whether the
/// caller has remembered the `#[serial]` attribute — and tests still should
/// mark themselves `#[serial]` so they coordinate with other lock-aware
/// guards. The lock is held for the guard's entire lifetime, so the original
/// directory is restored under the same lock that switched it.
///
/// The lock is acquired with `try_lock` rather than a blocking `lock`. The
/// underlying mutex is **non-reentrant**, so a single test that already holds
/// it — for example, one that has constructed an [`EnvSandbox`] — would
/// otherwise deadlock here. With `try_lock` the same misuse fails fast with
/// [`io::ErrorKind::WouldBlock`], turning a hung test into a finite,
/// debuggable failure. Tests should still pick one guard per test rather than
/// relying on this fail-fast.
#[derive(Debug)]
pub struct CwdGuard {
    original: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl CwdGuard {
    /// Switch the process cwd to `dir` for the lifetime of the guard.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::WouldBlock`] when the global env/sandbox lock
    /// is already held (typically because the test also constructed an
    /// [`EnvSandbox`] or another `CwdGuard`). Returns the underlying
    /// `io::Error` when the current directory cannot be read or when the
    /// chdir to `dir` fails.
    ///
    /// # Panics
    ///
    /// Panics if the env/sandbox mutex has been poisoned by a previous test
    /// panicking while holding it. Mirrors the behaviour of
    /// `env_sandbox_lock`.
    pub fn enter(dir: &Path) -> io::Result<Self> {
        let guard = match ENV_SANDBOX_LOCK.get_or_init(|| Mutex::new(())).try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                panic!("environment sandbox lock poisoned")
            }
        };
        let original = env::current_dir()?;
        env::set_current_dir(dir)?;
        Ok(Self {
            original,
            _guard: guard,
        })
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        // The lock is still held via `self._guard`, so the restore happens
        // atomically with respect to other lock-aware test helpers.
        let _ = env::set_current_dir(&self.original);
    }
}

#[cfg(test)]
mod cwd_guard_tests {
    use super::{CwdGuard, EnvSandbox};
    use serial_test::serial;
    use std::io;

    /// Pins the fail-fast contract: when the env/sandbox lock is already
    /// held, `CwdGuard::enter` must return `WouldBlock` immediately rather
    /// than block.
    #[test]
    #[serial]
    fn enter_reports_would_block_when_lock_held() {
        let sandbox = EnvSandbox::new().expect("create sandbox");
        let err =
            CwdGuard::enter(sandbox.path()).expect_err("CwdGuard must not acquire a held lock");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(sandbox);
    }
}
