//! Common test utilities.
//!
//! Provides helpers for manipulating environment variables and spinning up a
//! stub GraphQL server for integration tests.

use crate::api::{GraphQLClient, RetryConfig};
use vk::environment;

use std::env;
use std::ffi::OsString;
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

/// A temporary Git repository directory for tests.
///
/// Initialises a hermetic repo in a `tempfile::TempDir` and exposes builders
/// for the common shapes the `ref_parser` and `commands` test suites need:
/// branch-pointing HEAD, detached HEAD, `FETCH_HEAD` contents, and an `origin`
/// remote.
///
/// The fixture intentionally does *not* change the process working directory.
/// Tests that exercise code paths reading from the current directory (for
/// example, the public `repo_from_fetch_head` / `repo_from_origin` helpers)
/// should compose this with [`CwdGuard`] and mark themselves `#[serial]`.
pub struct GitRepoFixture {
    dir: tempfile::TempDir,
}

impl GitRepoFixture {
    /// Create a fixture whose HEAD is a symbolic ref to `branch`.
    ///
    /// No commit is required — `git symbolic-ref` is sufficient and keeps the
    /// fixture cheap.
    #[must_use]
    pub fn on_branch(branch: &str) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        // `-c init.defaultBranch=main` keeps behaviour stable on Git < 2.28.
        let status = std::process::Command::new("git")
            .args(["-c", "init.defaultBranch=main", "init"])
            .current_dir(dir.path())
            .output()
            .expect("git init");
        assert!(status.status.success(), "git init failed");

        let status = std::process::Command::new("git")
            .args(["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])
            .current_dir(dir.path())
            .output()
            .expect("git symbolic-ref");
        assert!(status.status.success(), "git symbolic-ref failed");

        Self { dir }
    }

    /// Create a fixture with a detached HEAD pointing at an empty commit.
    #[must_use]
    pub fn detached() -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");

        let status = std::process::Command::new("git")
            .args(["-c", "init.defaultBranch=main", "init"])
            .current_dir(dir.path())
            .output()
            .expect("git init");
        assert!(status.status.success(), "git init failed");

        for (key, value) in [("user.email", "test@test.com"), ("user.name", "Test")] {
            let status = std::process::Command::new("git")
                .args(["config", key, value])
                .current_dir(dir.path())
                .output()
                .expect("git config");
            assert!(status.status.success(), "git config {key} failed");
        }

        let status = std::process::Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ])
            .current_dir(dir.path())
            .output()
            .expect("git commit");
        assert!(status.status.success(), "git commit failed");

        let status = std::process::Command::new("git")
            .args(["checkout", "--detach"])
            .current_dir(dir.path())
            .output()
            .expect("git checkout --detach");
        assert!(status.status.success(), "git checkout --detach failed");

        Self { dir }
    }

    /// Configure an `origin` remote pointing at `url`.
    #[must_use]
    pub fn with_origin(self, url: &str) -> Self {
        let status = std::process::Command::new("git")
            .args(["remote", "add", "origin", url])
            .current_dir(self.dir.path())
            .output()
            .expect("git remote add");
        assert!(status.status.success(), "git remote add failed");
        self
    }

    /// Write `content` to the repository's `FETCH_HEAD` file.
    #[must_use]
    pub fn with_fetch_head(self, content: &str) -> Self {
        let git_dir = self.dir.path().join(".git");
        std::fs::write(git_dir.join("FETCH_HEAD"), content).expect("write FETCH_HEAD");
        self
    }

    /// Path to the repository's working directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// RAII guard that switches the process working directory and restores it on
/// drop.
///
/// The current working directory is process-global, so tests using `CwdGuard`
/// must run serially (typically via `#[serial_test::serial]`) to avoid racing
/// other tests.
pub struct CwdGuard {
    original: PathBuf,
}

impl CwdGuard {
    /// Switch the process cwd to `dir` for the lifetime of the guard.
    #[must_use]
    pub fn enter(dir: &Path) -> Self {
        let original = env::current_dir().expect("cwd");
        env::set_current_dir(dir).expect("chdir to fixture");
        Self { original }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original);
    }
}
