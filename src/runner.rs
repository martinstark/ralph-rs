//! Main runner orchestration layer.
//!
//! Coordinates the ralph loop: setup, delegation to specialized modules,
//! and overall session lifecycle management.

use crate::{
    analysis::IterationResult,
    config::Args,
    dry_run, init,
    iteration::{self, IterationContext},
    output, prd, retry,
    webhook::{self, EventType},
};
use anyhow::{bail, Context, Result};
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
            result = iteration::run(iteration, &ctx, &cancel_token) => {
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
