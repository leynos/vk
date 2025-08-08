//! Configuration loading helpers.
//!
//! Provides a wrapper around `ortho_config` that tolerates missing `reference`
//! fields by falling back to command-line values.

use figment::error::{Error as FigmentError, Kind as FigmentKind};
use ortho_config::{OrthoConfig, OrthoError, load_and_merge_subcommand_for};

fn missing_reference(err: &FigmentError) -> bool {
    // FigmentError yields its causes only by value; clone to inspect without ownership.
    err.clone()
        .into_iter()
        .any(|e| matches!(e.kind, FigmentKind::MissingField(ref f) if f == "reference"))
}

/// Load configuration for a set of CLI arguments, falling back when `reference`
/// is omitted.
///
/// # Errors
///
/// Returns an [`OrthoError`] if configuration gathering fails for reasons other
/// than a missing reference field.
#[expect(
    clippy::result_large_err,
    reason = "configuration loading errors can be verbose"
)]
pub fn load_with_reference_fallback<T>(cli_args: T) -> Result<T, OrthoError>
where
    T: OrthoConfig + serde::Serialize + Default + clap::CommandFactory + Clone,
{
    match load_and_merge_subcommand_for::<T>(&cli_args) {
        Ok(v) => Ok(v),
        Err(OrthoError::Gathering(e)) => {
            if missing_reference(&e) {
                Ok(cli_args)
            } else {
                Err(OrthoError::Gathering(e))
            }
        }
        Err(e) => Err(e),
    }
}
