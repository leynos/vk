//! Reading this repository's workflow files into the shape its contracts
//! assert against.
//!
//! The contracts need things the raw YAML does not hand over: which events a
//! workflow answers, which workflows run on behalf of a pull request, every
//! step of every job with its inputs, environment and condition, and every
//! place a secret is mentioned. Each is derived here so the assertions read as
//! claims rather than as parsing. Reading files, parsing text and interpreting
//! expressions are separate modules, so each can be tested without the others.

mod closure;
mod expression;
mod load;
mod model;
mod parse;

pub(crate) use closure::pull_request_closure;
pub(crate) use expression::{conjuncts, local_call_target, references_secret};
pub(crate) use load::{WorkflowError, load};
pub(crate) use model::{Job, Step, Workflow, jobs, steps};
pub(crate) use parse::parse_workflow;
