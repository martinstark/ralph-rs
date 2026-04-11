use anyhow::Result;
use clap::Parser;
use ralph_rs::{
    config::Args,
    output, prd, prompt,
    runner::{self, RunOutcome},
};
use std::{path::Path, process::ExitCode};

const INTERRUPTED_EXIT_CODE: u8 = 130;

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let args = Args::parse();

    if args.init {
        prd::generate_template(&args.prd)?;
        output::success(&format!("Created template PRD at {}", args.prd.display()));
        return Ok(ExitCode::SUCCESS);
    }

    if args.init_prompt {
        let path = Path::new("prompt.md");
        prompt::generate_prompt_template(path)?;
        output::success(&format!("Created prompt template at {}", path.display()));
        return Ok(ExitCode::SUCCESS);
    }

    Ok(exit_code_for_outcome(runner::run(args).await?))
}

fn exit_code_for_outcome(outcome: RunOutcome) -> ExitCode {
    match outcome {
        RunOutcome::Completed => ExitCode::SUCCESS,
        RunOutcome::Interrupted => ExitCode::from(INTERRUPTED_EXIT_CODE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_runs_exit_successfully() {
        assert_eq!(
            exit_code_for_outcome(RunOutcome::Completed),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn interrupted_runs_exit_with_code_130() {
        assert_eq!(
            exit_code_for_outcome(RunOutcome::Interrupted),
            ExitCode::from(INTERRUPTED_EXIT_CODE)
        );
    }
}
