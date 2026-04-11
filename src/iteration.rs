use crate::{
    analysis::{analyze_iteration_output, IterationResult, OutputAnalysisContext},
    claude::{self, ClaudeArgs, ClaudeOutcome, ClaudeResult},
    config::Args,
    git, output, prd, prompt, validation,
};
use anyhow::Result;
use chrono::{Local, Utc};
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
    if cancel_token.is_cancelled() {
        return Ok(IterationResult::Cancelled);
    }

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
        ctx.completion_marker,
    )?;

    let claude_args = ClaudeArgs {
        permission_mode: ctx.args.permission_mode.clone(),
        continue_session: ctx.args.continue_session,
        dangerously_skip_permissions: ctx.args.dangerously_skip_permissions,
        timeout_secs: ctx.args.timeout,
        project_dir: ctx.project_dir,
    };

    let result = claude::run_claude(&system_prompt, &claude_args, &log_path, cancel_token).await?;

    if should_cancel_iteration(&result, cancel_token) {
        return Ok(IterationResult::Cancelled);
    }

    if result.is_success() {
        output::success(&format!("Iteration {iteration} completed"));
    } else {
        output::warn(&format!("Iteration {iteration} exited with error"));
    }

    finalize_iteration_result(
        &result,
        ctx.completion_marker,
        Utc::now(),
        cancel_token,
        || {
            if git::is_git_repo() {
                validation::validate_prd_changes(&ctx.args.prd.to_string_lossy())
            } else {
                output::warn("Not a git repository - skipping PRD validation");
                Ok(())
            }
        },
    )
}

fn finalize_iteration_result<F>(
    result: &ClaudeResult,
    completion_marker: &str,
    observed_at: chrono::DateTime<Utc>,
    cancel_token: &CancellationToken,
    validate_prd_changes: F,
) -> Result<IterationResult>
where
    F: FnOnce() -> Result<()>,
{
    if should_cancel_iteration(result, cancel_token) {
        return Ok(IterationResult::Cancelled);
    }

    let validation_result = validate_prd_changes();

    if cancel_token.is_cancelled() {
        return Ok(IterationResult::Cancelled);
    }

    if let Err(e) = validation_result {
        output::error(&format!("PRD validation failed: {e}"));
        return Ok(IterationResult::Failed);
    }

    let analysis_ctx = OutputAnalysisContext {
        success: result.is_success(),
        completion_marker,
        observed_at,
    };
    Ok(classify_claude_result(result, &analysis_ctx, cancel_token))
}

fn classify_claude_result(
    result: &ClaudeResult,
    analysis_ctx: &OutputAnalysisContext<'_>,
    cancel_token: &CancellationToken,
) -> IterationResult {
    if should_cancel_iteration(result, cancel_token) {
        return IterationResult::Cancelled;
    }

    analyze_iteration_output(&result.output, analysis_ctx)
}

fn should_cancel_iteration(result: &ClaudeResult, cancel_token: &CancellationToken) -> bool {
    cancel_token.is_cancelled() || matches!(result.outcome, ClaudeOutcome::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use chrono::TimeZone;
    use std::cell::Cell;

    fn observed_at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 10, 0, 0, 0).single().unwrap()
    }

    fn analysis_ctx(success: bool) -> OutputAnalysisContext<'static> {
        OutputAnalysisContext {
            success,
            completion_marker: "DONE",
            observed_at: observed_at(),
        }
    }

    fn success_result() -> ClaudeResult {
        ClaudeResult {
            output: "DONE".to_string(),
            outcome: ClaudeOutcome::Success,
        }
    }

    #[test]
    fn classify_claude_result_returns_cancelled_without_analysis() {
        let result = ClaudeResult {
            output: "DONE".to_string(),
            outcome: ClaudeOutcome::Cancelled,
        };
        let cancel_token = CancellationToken::new();

        assert_eq!(
            classify_claude_result(&result, &analysis_ctx(false), &cancel_token),
            IterationResult::Cancelled
        );
    }

    #[test]
    fn classify_claude_result_still_detects_rate_limits_for_failures() {
        let result = ClaudeResult {
            output: "Error: rate limit exceeded".to_string(),
            outcome: ClaudeOutcome::Failure,
        };
        let cancel_token = CancellationToken::new();

        assert!(matches!(
            classify_claude_result(&result, &analysis_ctx(false), &cancel_token),
            IterationResult::RateLimit(_)
        ));
    }

    #[test]
    fn classify_claude_result_returns_cancelled_when_shutdown_requested() {
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();

        assert_eq!(
            classify_claude_result(&success_result(), &analysis_ctx(true), &cancel_token),
            IterationResult::Cancelled
        );
    }

    #[test]
    fn finalize_iteration_result_skips_validation_when_shutdown_requested() {
        let cancel_token = CancellationToken::new();
        cancel_token.cancel();
        let validate_called = Cell::new(false);

        let result = finalize_iteration_result(
            &success_result(),
            "DONE",
            observed_at(),
            &cancel_token,
            || {
                validate_called.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(result, IterationResult::Cancelled);
        assert!(!validate_called.get());
    }

    #[test]
    fn finalize_iteration_result_returns_cancelled_when_shutdown_requested_during_validation() {
        let cancel_token = CancellationToken::new();

        let result = finalize_iteration_result(
            &success_result(),
            "DONE",
            observed_at(),
            &cancel_token,
            || {
                cancel_token.cancel();
                Err(anyhow!("validation interrupted"))
            },
        )
        .unwrap();

        assert_eq!(result, IterationResult::Cancelled);
    }

    #[test]
    fn finalize_iteration_result_returns_failed_for_validation_error_without_shutdown() {
        let cancel_token = CancellationToken::new();

        let result = finalize_iteration_result(
            &success_result(),
            "DONE",
            observed_at(),
            &cancel_token,
            || Err(anyhow!("validation failed")),
        )
        .unwrap();

        assert_eq!(result, IterationResult::Failed);
    }
}
