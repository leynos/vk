//! Terminal output helpers for the `vk` subcommands.
//!
//! These functions print review threads, review decisions and banners, and
//! decide whether a failed write means the reader closed the pipe, in which
//! case callers stop printing quietly.

use crate::printer::{print_reviews, write_thread};
use crate::reviews::{PullRequestReview, latest_reviews};
use crate::{ReviewThread, VkError};
use std::any::Any;
use std::io::ErrorKind;
use termimad::MadSkin;
use tracing::error;

/// Print a review thread to stdout.
///
/// This simply calls [`write_thread`] with a locked `stdout` handle.
pub(super) fn print_thread(skin: &MadSkin, thread: &ReviewThread) -> Result<(), VkError> {
    write_thread(std::io::stdout().lock(), skin, thread).map_err(map_printer_error)
}

/// Convert a printer error into the command error type.
pub(super) fn map_printer_error(err: anyhow::Error) -> VkError {
    if let Some(io) = err.downcast_ref::<std::io::Error>() {
        return VkError::Io(Box::new(std::io::Error::new(io.kind(), err)));
    }
    VkError::Io(Box::new(std::io::Error::other(err)))
}

/// Inspect a print `result`, returning true when a broken pipe is detected so
/// callers stop further output, or false when normal processing should
/// continue.
///
/// The `result` captures output failures and `label` provides context for
/// logging. Broken pipe checks apply to `std::io::Error` and
/// `termimad::Error` values.
pub(super) fn handle_print_result<E>(result: Result<(), E>, label: &str) -> bool
where
    E: std::fmt::Display + Any,
{
    let Err(err) = result else {
        return false;
    };
    let any = &err as &dyn Any;
    if any
        .downcast_ref::<std::io::Error>()
        .is_some_and(|io| is_broken_pipe_kind(io.kind()))
    {
        return true;
    }
    if any
        .downcast_ref::<termimad::Error>()
        .is_some_and(|term_err| match term_err {
            termimad::Error::IO(io) => is_broken_pipe_kind(io.kind()),
            termimad::Error::InsufficientWidth(_) => false,
        })
    {
        return true;
    }
    error!("error printing {label}: {err}");
    false
}

/// Return whether an error chain contains a broken-pipe I/O error.
pub(super) fn caused_by_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| is_broken_pipe_kind(io.kind()))
    })
}

/// Return whether an I/O error kind indicates a closed output pipe.
pub(super) fn is_broken_pipe_kind(kind: ErrorKind) -> bool {
    kind == ErrorKind::BrokenPipe
}

/// Print a banner and report whether output stopped because of a broken pipe.
pub(super) fn handle_banner<F>(print: F, label: &str) -> bool
where
    F: FnOnce() -> std::io::Result<()>,
{
    if let Err(e) = print() {
        if is_broken_pipe_kind(e.kind()) {
            return true;
        }
        error!("error printing {label} banner: {e}");
    }
    false
}

/// Print the latest review decisions, stopping on a broken pipe.
pub(super) fn print_reviews_block(skin: &MadSkin, reviews: Vec<PullRequestReview>) -> bool {
    let latest = latest_reviews(reviews);
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if let Err(e) = print_reviews(&mut handle, skin, &latest) {
        if caused_by_broken_pipe(&e) {
            return true;
        }
        error!("error printing review: {e}");
    }
    false
}

/// Print review threads, stopping on a broken pipe.
pub(super) fn print_threads_block(skin: &MadSkin, threads: Vec<ReviewThread>) -> bool {
    for thread in threads {
        if let Err(e) = print_thread(skin, &thread) {
            if matches!(e, VkError::Io(ref inner) if is_broken_pipe_kind(inner.kind())) {
                return true;
            }
            error!("error printing thread: {e}");
        }
    }
    false
}
