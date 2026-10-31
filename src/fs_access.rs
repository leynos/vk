//! Capability-based access to files named by the user or reported by Git.
//!
//! The command line accepts a transcript path and Git reports where
//! `FETCH_HEAD` lives, so neither path is known to be beneath a directory this
//! program already holds. Each helper opens the path's parent directory as a
//! [`Dir`] and acts on the file name beneath it, so file access goes through a
//! directory capability rather than `std::fs`. This is the one production
//! module that calls `ambient_authority`; integration-test helpers in
//! `tests/support/capfs.rs` call it too. The helper does not confine a path to a
//! preselected root: it acquires authority over whichever parent it is given.
//!
//! One behaviour differs from `std::fs`: `cap-std` resolves symlinks within the
//! opened directory and refuses one whose target escapes it, including an
//! absolute target. A transcript path or `FETCH_HEAD` that is itself such a
//! symlink now fails, and the callers treat that as they treat any I/O error.
//! The restriction is intentional.

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

/// Write `contents` to the file at `path`, creating or truncating it.
///
/// Test-only convenience over [`create_file`].
///
/// # Errors
///
/// Returns the I/O error from [`create_file`] or from the write.
#[cfg(test)]
pub(crate) fn write_string(path: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;

    create_file(path)?.write_all(contents.as_bytes())
}

#[cfg(test)]
mod tests {
    //! Tests for the capability-based file helpers.

    use std::io::{ErrorKind, Write};
    use std::path::Path;

    use super::{create_file, parent_and_name, read_to_string, write_string};

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
    fn a_missing_file_is_not_found() {
        let dir = tempfile::tempdir().expect("temporary directory");

        let error = read_to_string(&dir.path().join("absent")).expect_err("absent file");

        assert_eq!(error.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn a_missing_parent_directory_fails_both_helpers() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("no-such-dir").join("note.txt");

        assert_eq!(
            create_file(&path).expect_err("no parent").kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            read_to_string(&path).expect_err("no parent").kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn a_directory_cannot_be_created_as_a_file() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let target = dir.path().join("sub");
        std::fs::create_dir(&target).expect("directory");

        assert!(create_file(&target).is_err());
    }

    #[test]
    fn invalid_utf8_contents_fail_the_read() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("bytes.bin");
        create_file(&path)
            .expect("create")
            .write_all(&[0xff, 0xfe])
            .expect("write");

        let error = read_to_string(&path).expect_err("not UTF-8");

        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn a_path_without_a_file_name_is_refused_by_both_helpers() {
        let root = Path::new("/");

        assert_eq!(
            create_file(root).expect_err("no name").kind(),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            read_to_string(root).expect_err("no name").kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn a_bare_relative_name_falls_back_to_the_current_directory() {
        let (parent, name) = parent_and_name(Path::new("note.txt")).expect("split");

        assert_eq!(parent, Path::new("."));
        assert_eq!(name, "note.txt");
    }

    #[test]
    fn creating_an_existing_file_truncates_it() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("note.txt");
        write_string(&path, "a long first body").expect("first write");

        write_string(&path, "short").expect("second write");

        assert_eq!(read_to_string(&path).expect("read"), "short");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_whose_target_escapes_the_directory_is_refused() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let target = dir.path().join("target.txt");
        write_string(&target, "inside").expect("write target");
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        // `cap-std` treats an absolute symlink target as leaving the opened
        // directory, which `std::fs` would follow. The restriction is intended.
        assert!(read_to_string(&link).is_err());
    }
}
