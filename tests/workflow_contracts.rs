//! Contracts over this repository's own workflow files.
//!
//! Who may contact CodeScene, who owns the coverage upload, where each lane
//! runs and what it may bill, and how Markdown is linted. None of these is
//! visible in a green run: a pull-request lane that reaches CodeScene, a token
//! exported into a whole job, a lane moved back to a runner nobody pays for,
//! or a linter taken from the runner image all pass CI while breaking the
//! rule.
//!
//! Every contract derives its subject from the workflows' own triggers and
//! calls rather than from a list of file names. A contract keyed on names
//! passes unchanged when a workflow is added, which is precisely when the
//! question is being asked again.
//!
//! One test binary rather than one per contract, so the reader they share is
//! compiled once and every item in it is used by the binary that compiles it.

#[path = "workflow_contracts/codescene.rs"]
mod codescene;
#[path = "workflow_contracts/markdown_lint.rs"]
mod markdown_lint;
#[path = "workflow_contracts/placement.rs"]
mod placement;
#[path = "workflow_contracts/placement_tests.rs"]
mod placement_tests;
#[path = "workflow_contracts/properties.rs"]
mod properties;
#[path = "workflow_contracts/publisher.rs"]
mod publisher;
#[path = "workflow_contracts/pull_request_concurrency.rs"]
mod pull_request_concurrency;
#[path = "workflow_contracts/pull_request_lanes.rs"]
mod pull_request_lanes;
#[path = "support/workflows/mod.rs"]
mod reader;
#[path = "workflow_contracts/reader_tests.rs"]
mod reader_tests;
#[path = "workflow_contracts/registry.rs"]
mod registry;
#[path = "workflow_contracts/rule_tests.rs"]
mod rule_tests;
#[path = "workflow_contracts/uploader.rs"]
mod uploader;

use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;
use rstest::fixture;

use crate::reader::{Workflow, WorkflowError, load};

/// Every workflow this repository declares, parsed once per contract.
///
/// Ambient authority is taken here, at the boundary, and nowhere else: the
/// reader is handed the opened directory, so it can be handed a temporary one
/// in its own tests just as readily.
#[fixture]
fn repository() -> Result<Vec<Workflow>, WorkflowError> {
    let directory = Dir::open_ambient_dir(
        concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows"),
        ambient_authority(),
    )
    .map_err(|source| WorkflowError::List { source })?;
    load(&directory)
}
