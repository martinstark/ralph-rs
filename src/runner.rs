use crate::{
    analysis::{analyze_iteration_output, IterationResult, OutputAnalysisContext},
    claude::{self, ClaudeArgs},
    config::Args,
    dry_run, git, init, output, prd, prompt, retry, validation,
    webhook::{self, EventType},
};
use anyhow::{bail, Context, Result};
use chrono::Local;
use tokio::signal;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

const MAX_CONSECUTIVE_FAILURES: u32 = 3;

pub async fn run(args: Args) -> Result<()> {
    if !args.prd.exists() {
        output::error(&format!("PRD file not found: {}", args.prd.display()));
        output::log("Run 'ralph --init' to create a template, or specify path with -p");
        bail!("PRD file not found");
    }

    let prd = prd::Prd::load(&args.prd)?;

    if args.dry_run {
        return dry_run::run(&args, &prd);
    }

    let project_dir = args
        .prd
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let progress_path = project_dir.join("progress.txt");
    let ralph_dir = project_dir.join(".ralph");
    let logs_dir = ralph_dir.join("logs");

    std::fs::create_dir_all(&logs_dir)
        .context("Failed to create .ralph/logs directory")?;

    if !progress_path.exists() {
        std::fs::write(
            &progress_path,
            "# Ralph Progress Log\n\nAppend-only log of session activity.\n\n---\n\n",
        )?;
    }

    if !args.skip_init {
        init::run_init_phase(&prd, &args.prd, &progress_path)?;
    }

    if let Some(ref url) = args.webhook {
        webhook::send_webhook(url, EventType::SessionStart, &format!("Starting session for {}", prd.project.name));
    }

    let completion_marker = args
        .completion_marker
        .as_ref()
        .unwrap_or(&prd.completion.marker);

    output::section("Phase 2: Ralph Loop");
    output::log(&format!("PRD file: {}", args.prd.display()));
    output::log(&format!("Progress file: {}", progress_path.display()));
    if let Some(ref prompt_path) = args.prompt {
        output::log(&format!("Custom prompt: {}", prompt_path.display()));
    }
    output::log(&format!("Completion marker: {completion_marker}"));
    output::log(&format!("Permission mode: {}", args.permission_mode));
    output::log(&format!(
        "Session mode: {}",
        if args.continue_session {
            "continue (preserves context)"
        } else {
            "print (fresh each iteration)"
        }
    ));
    if args.max_iterations > 0 {
        output::log(&format!("Max iterations: {}", args.max_iterations));
    }
    if args.max_iteration_errors > 0 {
        output::log(&format!("Max iteration errors: {}", args.max_iteration_errors));
    }
    println!();

    let start_time = std::time::Instant::now();
    let mut iteration: u32 = 0;
    let mut consecutive_failures: u32 = 0;
    let mut error_tracker = retry::IterationErrorTracker::new(args.max_iteration_errors);

    loop {
        iteration += 1;

        let current_prd = prd::Prd::load(&args.prd)?;

        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();

        let ctx = IterationContext {
            args: &args,
            prd: &current_prd,
            progress_path: &progress_path,
            logs_dir: &logs_dir,
            completion_marker,
            project_dir,
            prompt_path: args.prompt.as_deref(),
        };

        tokio::select! {
            _ = signal::ctrl_c() => {
                cancel_token_clone.cancel();
                println!();
                output::warn(&format!("Ralph loop interrupted after {iteration} iterations"));
                let duration = start_time.elapsed();
                output::log(&format!("Total runtime: {}", output::format_duration(duration)));
                return Ok(());
            }
            result = run_iteration(iteration, &ctx, &cancel_token) => {
                match result {
                    Ok(IterationResult::Continue) => {
                        consecutive_failures = 0;
                    }
                    Ok(IterationResult::Complete) => {
                        println!();
                        output::separator();
                        output::success("Completion marker found! Ralph loop finished.");
                        output::separator();
                        let duration = start_time.elapsed();
                        output::log(&format!("Total iterations: {iteration}"));
                        output::log(&format!("Total runtime: {}", output::format_duration(duration)));
                        output::log(&format!("Logs saved to: {}", logs_dir.display()));
                        if let Some(ref url) = args.webhook {
                            webhook::send_webhook(url, EventType::SessionComplete, &format!("Session complete after {iteration} iterations"));
                        }
                        return Ok(());
                    }
                    Ok(IterationResult::RateLimit) => {
                        output::error("Rate limit detected. Waiting 60s before retry...");
                        sleep(Duration::from_secs(60)).await;
                    }
                    Ok(IterationResult::LoopDetected) => {
                        output::warn("Loop detection: Agent appears blocked");
                        handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                        handle_failure(&mut consecutive_failures, iteration, start_time, &logs_dir, args.webhook.as_deref())?;
                    }
                    Ok(IterationResult::Failed) => {
                        handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                        handle_failure(&mut consecutive_failures, iteration, start_time, &logs_dir, args.webhook.as_deref())?;
                    }
                    Err(e) => {
                        output::error(&format!("Iteration error: {e:#}"));
                        handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                        handle_failure(&mut consecutive_failures, iteration, start_time, &logs_dir, args.webhook.as_deref())?;
                    }
                }
            }
        }

        if args.max_iterations > 0 && iteration >= args.max_iterations {
            println!();
            output::warn(&format!("Max iterations ({}) reached", args.max_iterations));
            let duration = start_time.elapsed();
            output::log(&format!("Total runtime: {}", output::format_duration(duration)));
            output::log(&format!("Logs saved to: {}", logs_dir.display()));
            return Ok(());
        }

        println!();
        output::dim(&format!("Waiting {}s before next iteration...", args.delay));
        sleep(Duration::from_secs(args.delay)).await;
        println!();
    }
}


/// Handles failure by incrementing counter and checking if max failures reached.
/// Returns Err if too many consecutive failures, Ok(()) otherwise.
fn handle_failure(
    consecutive_failures: &mut u32,
    iteration: u32,
    start_time: std::time::Instant,
    logs_dir: &std::path::Path,
    webhook_url: Option<&str>,
) -> Result<()> {
    *consecutive_failures += 1;
    if *consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
        println!();
        output::separator();
        output::error(&format!(
            "Too many consecutive failures ({consecutive_failures})"
        ));
        output::error("The agent may be stuck. Review logs and PRD.");
        output::separator();
        let duration = start_time.elapsed();
        output::log(&format!("Total iterations: {iteration}"));
        output::log(&format!("Total runtime: {}", output::format_duration(duration)));
        output::log(&format!("Logs saved to: {}", logs_dir.display()));
        if let Some(url) = webhook_url {
            webhook::send_webhook(url, EventType::SessionFailed, &format!("Session failed after {iteration} iterations: too many consecutive failures"));
        }
        bail!("Too many consecutive failures");
    }
    Ok(())
}

fn handle_iteration_error(
    tracker: &mut retry::IterationErrorTracker,
    prd_path: &std::path::Path,
    current_prd: &prd::Prd,
) -> Result<()> {
    if !tracker.is_enabled() {
        return Ok(());
    }

    if let Some(feature_id) = retry::get_current_feature_id(current_prd) {
        let count = tracker.record_error(&feature_id);

        if tracker.should_block(&feature_id) {
            retry::update_feature_status_to_blocked(prd_path, &feature_id)?;
        } else {
            output::warn(&format!("Feature '{}' error count: {}", feature_id, count));
        }
    }

    Ok(())
}

struct IterationContext<'a> {
    args: &'a Args,
    prd: &'a prd::Prd,
    progress_path: &'a std::path::Path,
    logs_dir: &'a std::path::Path,
    completion_marker: &'a str,
    project_dir: &'a std::path::Path,
    prompt_path: Option<&'a std::path::Path>,
}

async fn run_iteration(
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

