//! Injected environments for configuration-merge integration tests.
//!
//! Builds an [`ortho_config::MapEnv`] that points `VK_CONFIG_PATH` at a
//! temporary configuration file, so the tests never change the process
//! environment or working directory.

use ortho_config::MapEnv;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

#[path = "capfs.rs"]
mod capfs;

/// Write `content` to a temporary `.vk.toml` and return its directory and path.
pub fn write_config(content: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("create config dir");
    let path = dir.path().join(".vk.toml");
    capfs::write_file(&path, content).expect("write config");
    (dir, path)
}

/// Injected environment for one configuration-merge case.
///
/// Owns the temporary directory holding the configuration file, so the file
/// outlives every load made through [`MergeEnv::env`].
pub struct MergeEnv {
    _dir: TempDir,
    env: Arc<MapEnv>,
}

impl MergeEnv {
    /// Write `config`, then build an environment holding `VK_CONFIG_PATH` and
    /// every `Some` pair in `env`. A `None` pair is simply absent.
    ///
    /// The environment's working directory is the configuration directory, so
    /// discovery finds the generated `.vk.toml` and never reads the process's
    /// working directory.
    ///
    /// # Examples
    /// ```rust,ignore
    /// use crate::env_support::MergeEnv;
    ///
    /// let merge_env = MergeEnv::new("[cmds.pr]\nreference = \"ref\"", &[("VK_REPO", None)]);
    /// let merged = cli.load_and_merge_with_env(merge_env.env()).expect("merge");
    /// ```
    pub fn new(config: &str, env: &[(&str, Option<&str>)]) -> Self {
        let (dir, config_path) = write_config(config);
        let base = MapEnv::new()
            .with_current_dir(dir.path())
            .with_var("VK_CONFIG_PATH", config_path.as_os_str());
        let map = env
            .iter()
            .filter_map(|(key, value)| value.map(|value| (*key, value)))
            .fold(base, |map, (key, value)| map.with_var(key, value));
        Self {
            _dir: dir,
            env: Arc::new(map),
        }
    }

    /// Return a shared handle to the injected environment.
    pub fn env(&self) -> Arc<MapEnv> {
        Arc::clone(&self.env)
    }
}
