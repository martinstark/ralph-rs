use crate::{
    analysis::{analyze_iteration_output, IterationResult, OutputAnalysisContext},
    claude::{self, ClaudeArgs},
    config::Args,
    git, output, prd, prompt, validation,
};
use anyhow::Result;
use chrono::Local;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub struct IterationContext<'a> {
    pub args: &'a Args,
    pub prd: &'a prd::Prd,
    pub progress_path: &'a Path,
    pub logs_dir: &'a Path,
    pub completion_marker: &'a str,
    pub project_dir: &'a Path,
    pub prompt_path: Option<&'a Path>,
}

pub async fn run(
    iteration: u32,
    ctx: &IterationContext<'_>,
    cancel_token: &CancellationToken,
) -> Result<IterationResult> {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    output::log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    output::log(&format!("Iteration {iteration} - {timestamp}"));
    output::log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    let log_filename = format!(
        "{}-iteration-{}.log",
        Local::now().format("%Y%m%d-%H%M%S"),
        iteration
    );
    let log_path = ctx.logs_dir.join(log_filename);

    let system_prompt = prompt::get_system_prompt(
        ctx.prompt_path,
        ctx.prd,
        &ctx.args.prd,
        ctx.progress_path,
    )?;

    let claude_args = ClaudeArgs {
        permission_mode: ctx.args.permission_mode.clone(),
        continue_session: ctx.args.continue_session,
        dangerously_skip_permissions: ctx.args.dangerously_skip_permissions,
        timeout_secs: ctx.args.timeout,
        project_dir: ctx.project_dir,
    };

    let result = claude::run_claude(&system_prompt, &claude_args, &log_path, cancel_token).await?;

    if result.success {
        output::success(&format!("Iteration {iteration} completed"));
    } else {
        output::warn(&format!("Iteration {iteration} exited with error"));
    }

    if git::is_git_repo() {
        if let Err(e) = validation::validate_prd_changes(&ctx.args.prd.to_string_lossy()) {
            output::error(&format!("PRD validation failed: {e}"));
            return Ok(IterationResult::Failed);
        }
    } else {
        output::warn("Not a git repository - skipping PRD validation");
    }

    let analysis_ctx = OutputAnalysisContext {
        success: result.success,
        completion_marker: ctx.completion_marker,
    };
    Ok(analyze_iteration_output(&result.output, &analysis_ctx))
}
