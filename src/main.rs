//! CLI entry point for the `vk` tool.

use clap::{Parser, Subcommand};
use log::{error, warn};
use termimad::MadSkin;
use vk::cli_args::{GlobalArgs, IssueArgs, PrArgs};
use vk::{
    VkError, build_graphql_client, fetch_issue, fetch_review_threads, load_with_reference_fallback,
    locale_is_utf8, parse_issue_reference, parse_pr_reference,
    printer::{print_end_banner, print_summary, print_thread, summarize_files},
    reviews::{fetch_reviews, latest_reviews, print_reviews},
};

#[derive(Subcommand, Clone, Debug)]
enum Commands {
    /// Show unresolved pull request comments
    Pr(PrArgs),
    /// Read a GitHub issue
    Issue(IssueArgs),
}

#[derive(Parser)]
#[command(
    name = "vk",
    about = "View Komments - show unresolved PR comments",
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[command(flatten)]
    global: GlobalArgs,
}

async fn run_pr(args: PrArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_pr_reference(reference, global.repo.as_deref())?;
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        warn!("GITHUB_TOKEN not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        warn!("terminal locale is not UTF-8; emojis may not render correctly");
    }

    let client = build_graphql_client(&token, global.transcript.as_ref())?;
    let threads = fetch_review_threads(&client, &repo, number).await?;
    let reviews = fetch_reviews(&client, &repo, number).await?;
    if threads.is_empty() {
        println!("No unresolved comments.");
        return Ok(());
    }

    let summary = summarize_files(&threads);
    print_summary(&summary);

    let skin = MadSkin::default();
    let latest = latest_reviews(reviews);
    print_reviews(&skin, &latest);

    for t in threads {
        if let Err(e) = print_thread(&skin, &t) {
            error!("error printing thread: {e}");
        }
    }
    print_end_banner();
    Ok(())
}

async fn run_issue(args: IssueArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_issue_reference(reference, global.repo.as_deref())?;
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        warn!("GITHUB_TOKEN not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        warn!("terminal locale is not UTF-8; emojis may not render correctly");
    }

    let client = build_graphql_client(&token, global.transcript.as_ref())?;
    let issue = fetch_issue(&client, &repo, number).await?;

    let skin = MadSkin::default();
    println!("\x1b[1m{}\x1b[0m", issue.title);
    skin.print_text(&issue.body);
    println!();
    Ok(())
}

#[allow(clippy::result_large_err, reason = "VkError variants are small")]
#[tokio::main]
async fn main() -> Result<(), VkError> {
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
