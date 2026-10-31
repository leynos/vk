//! Directory-capability file helpers for integration tests.
//!
//! Tests write fixtures at paths they build from a temporary directory. The
//! helper opens the path's parent as a [`cap_std::fs::Dir`] and acts on the file
//! name beneath it, so test code does not call `std::fs`.

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use std::path::Path;

/// Open the directory that holds `path` as a capability.
fn parent_dir(path: &Path) -> Dir {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Dir::open_ambient_dir(parent, ambient_authority()).expect("open parent directory")
}

/// Return the file name of `path`.
fn file_name(path: &Path) -> &std::ffi::OsStr {
    path.file_name().expect("path names a file")
}

/// Write `content` to the file at `path`, creating or truncating it.
pub fn write_file(path: &Path, content: &str) {
    parent_dir(path)
        .write(file_name(path), content)
        .expect("write file");
}
