use anyhow::Result;
use clap::Parser;
use ralph_rs::{config::Args, output, prd, runner};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle --init flag
    if args.init {
        prd::generate_template(&args.prd)?;
        output::success(&format!("Created template PRD at {}", args.prd.display()));
        return Ok(());
    }

    // Run the main Ralph loop
    runner::run(args).await
}
