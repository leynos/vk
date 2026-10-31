//! Directory-capability file helpers for integration tests.
//!
//! Tests write fixtures at paths they build from a temporary directory. The
//! helper opens the path's parent as a [`cap_std::fs::Dir`] and acts on the file
//! name beneath it, so test code does not call `std::fs`. Failures are returned,
//! not raised, so a shared fixture never panics on its own; the calling test
//! decides how to report them.

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use std::io;
use std::path::Path;

/// Write `content` to the file at `path`, creating or truncating it.
///
/// # Errors
///
/// Returns the I/O error when `path` has no file name, its parent directory
/// cannot be opened, or the file cannot be written.
pub fn write_file(path: &Path, content: &str) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} does not name a file", path.display()),
        )
    })?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Dir::open_ambient_dir(parent, ambient_authority())?.write(name, content)
}
