//! Capability-based access to files named by the user or reported by Git.
//!
//! The command line accepts a transcript path and Git reports where
//! `FETCH_HEAD` lives, so neither path is known to be beneath a directory this
//! program already holds. Each helper opens the path's parent directory as a
//! [`Dir`] and acts on the file name beneath it, so file access goes through a
//! directory capability rather than `std::fs`. This is the one place that calls
//! `ambient_authority`.

use std::io::{self, Read};
use std::path::Path;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, File};

/// Split `path` into the directory that holds it and the file name.
fn parent_and_name(path: &Path) -> io::Result<(&Path, &std::ffi::OsStr)> {
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
    Ok((parent, name))
}

/// Open the directory that holds `path` as a capability.
fn open_parent(parent: &Path) -> io::Result<Dir> {
    Dir::open_ambient_dir(parent, ambient_authority())
}

/// Create (or truncate) the file at `path` for writing.
///
/// # Errors
///
/// Returns the I/O error when the parent directory cannot be opened or the file
/// cannot be created.
pub(crate) fn create_file(path: &Path) -> io::Result<File> {
    let (parent, name) = parent_and_name(path)?;
    open_parent(parent)?.create(name)
}

/// Read the whole file at `path` into a string.
///
/// # Errors
///
/// Returns the I/O error when the parent directory or the file cannot be
/// opened, or when the contents are not valid UTF-8.
pub(crate) fn read_to_string(path: &Path) -> io::Result<String> {
    let (parent, name) = parent_and_name(path)?;
    let mut contents = String::new();
    open_parent(parent)?
        .open(name)?
        .read_to_string(&mut contents)?;
    Ok(contents)
}

#[cfg(test)]
mod tests {
    //! Tests for the capability-based file helpers.

    use std::io::Write;

    use super::{create_file, read_to_string};

    #[test]
    fn a_file_round_trips_through_its_parent_directory() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("note.txt");
        let mut file = create_file(&path).expect("create");
        file.write_all(b"hello").expect("write");
        drop(file);

        assert_eq!(read_to_string(&path).expect("read"), "hello");
    }

    #[test]
    fn a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().expect("temporary directory");

        assert!(read_to_string(&dir.path().join("absent")).is_err());
    }

    #[test]
    fn a_path_without_a_file_name_is_refused() {
        let error = create_file(std::path::Path::new("/")).expect_err("no file name");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
