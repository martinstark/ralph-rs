use anyhow::Result;
use clap::Parser;
use ralph_rs::{config::Args, output, prd, prompt, runner};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle --init flag
    if args.init {
        prd::generate_template(&args.prd)?;
        output::success(&format!("Created template PRD at {}", args.prd.display()));
        return Ok(());
    }

    // Handle --init-prompt flag
    if args.init_prompt {
        let path = Path::new("prompt.md");
        prompt::generate_prompt_template(path)?;
        output::success(&format!("Created prompt template at {}", path.display()));
        return Ok(());
    }

    // Run the main Ralph loop
    runner::run(args).await
}
