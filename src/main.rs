//! Entry point for the `vk` command line tool.
//!
//! Parses CLI arguments and dispatches to library functions.

use clap::Parser;
use vk::{
    Cli, Commands, GlobalArgs, IssueArgs, PrArgs, load_with_reference_fallback, run_issue, run_pr,
};
#[allow(clippy::result_large_err, reason = "VkError is large")]
#[tokio::main]
async fn main() -> Result<(), vk::VkError> {
    env_logger::init();
    let cli = Cli::parse();
    let mut global = GlobalArgs::load_from_iter(std::env::args_os().take(1))?;
    global.merge(cli.global);
    match cli.command {
        Commands::Pr(pr_cli) => {
            let args = load_with_reference_fallback::<PrArgs>(pr_cli.clone())?;
            run_pr(args, &global).await
        }
        Commands::Issue(issue_cli) => {
            let args = load_with_reference_fallback::<IssueArgs>(issue_cli.clone())?;
            run_issue(args, &global).await
        }
    }
}
