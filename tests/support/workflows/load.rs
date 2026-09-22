//! Reading every workflow in a directory, and the ways that can fail.
//!
//! The directory arrives as a capability opened by the caller, so this module
//! never takes ambient authority and a test can hand it a temporary directory
//! as readily as the repository's own. Every failure is returned rather than
//! panicked on, and none is discarded: a workflow that cannot be read or
//! parsed is an error, not an absence, because a contract that silently
//! inspects four workflows where there are five reports success for the one
//! it never saw.

use camino::Utf8Path;
use cap_std::fs_utf8::Dir;

use super::model::Workflow;
use super::parse::parse_workflow;

/// Why the workflows could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WorkflowError {
    /// The directory, or one of its entries, could not be listed.
    #[error("the workflow directory cannot be listed: {source}")]
    List {
        /// The underlying failure.
        source: std::io::Error,
    },
    /// A workflow file could not be read.
    #[error("{file} cannot be read: {source}")]
    Read {
        /// The file that failed.
        file: String,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// A workflow file is not YAML, or repeats a key within one mapping.
    #[error("{file} is not valid YAML: {source}")]
    Parse {
        /// The file that failed.
        file: String,
        /// The parser's complaint.
        source: serde_norway::Error,
    },
    /// A workflow file is YAML, but not a mapping.
    #[error("{file} is not a mapping, so it declares no workflow")]
    Shape {
        /// The file that failed.
        file: String,
    },
    /// The directory holds no workflow files at all.
    #[error("the workflow directory declares no workflows")]
    Empty,
    /// A job calls a workflow in this repository that does not exist.
    #[error("{caller} calls {target}, which is not a workflow in this repository")]
    UnresolvedCall {
        /// The calling job's coordinate.
        caller: String,
        /// The file the call names.
        target: String,
    },
}

/// Return whether a file name is a workflow's.
///
/// Compared through camino's extension reader rather than by suffix: a suffix
/// test is case-sensitive, and GitHub reads `.YML` as readily as `.yml`, so a
/// workflow named that way would be skipped in silence.
fn is_workflow_file(name: &str) -> bool {
    Utf8Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml"))
}

/// Return the name of every workflow file in `directory`, sorted.
fn workflow_names(directory: &Dir) -> Result<Vec<String>, WorkflowError> {
    let mut names = Vec::new();
    for entry in directory
        .entries()
        .map_err(|source| WorkflowError::List { source })?
    {
        let name = entry
            .and_then(|found| found.file_name())
            .map_err(|source| WorkflowError::List { source })?;
        if is_workflow_file(&name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Return one workflow, read through the directory capability and parsed.
fn workflow_at(directory: &Dir, file: &str) -> Result<Workflow, WorkflowError> {
    let text = directory
        .read_to_string(file)
        .map_err(|source| WorkflowError::Read {
            file: file.to_owned(),
            source,
        })?;
    parse_workflow(file, &text)
}

/// Return every workflow in `directory`, parsed.
///
/// ```ignore
/// let directory = Dir::open_ambient_dir(".github/workflows", ambient_authority())?;
/// let workflows = load(&directory)?;
/// ```
///
/// # Errors
///
/// Returns the first listing, reading or parsing failure, and
/// [`WorkflowError::Empty`] when the directory holds no workflow at all, since
/// every contract over an empty set passes.
pub(crate) fn load(directory: &Dir) -> Result<Vec<Workflow>, WorkflowError> {
    let names = workflow_names(directory)?;
    if names.is_empty() {
        return Err(WorkflowError::Empty);
    }
    names
        .iter()
        .map(|name| workflow_at(directory, name))
        .collect()
}
