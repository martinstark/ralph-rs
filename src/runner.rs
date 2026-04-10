//! Main runner orchestration layer.
//!
//! Coordinates the ralph loop: setup, delegation to specialized modules,
//! and overall session lifecycle management.

use crate::{
    analysis::IterationResult,
    config::{self, Args},
    dry_run, init,
    iteration::{self, IterationContext},
    output, prd,
    rate_limit::{self, RateLimitInfo, RetryDecision, RetryPolicy, RetryReason},
    retry,
    webhook::{self, EventType},
};
use anyhow::{bail, Context, Result};
use chrono::{Local, Utc};
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
    let completion_marker = config::resolve_completion_marker(args.completion_marker.as_deref())?;

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

    std::fs::create_dir_all(&logs_dir).context("Failed to create .ralph/logs directory")?;

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
        webhook::send_webhook(
            url,
            EventType::SessionStart,
            &format!("Starting session for {}", prd.project.name),
        );
    }

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
        output::log(&format!(
            "Max iteration errors: {}",
            args.max_iteration_errors
        ));
    }
    println!();

    let start_time = std::time::Instant::now();
    let mut iteration: u32 = 0;
    let mut consecutive_failures: u32 = 0;
    let mut error_tracker = retry::IterationErrorTracker::new(args.max_iteration_errors);
    let mut rate_limit_state = rate_limit::RateLimitState::default();
    let rate_limit_policy = RetryPolicy {
        fallback_seconds: args.rate_limit_fallback_seconds,
        reset_buffer_seconds: args.rate_limit_buffer_seconds,
        post_reset_max_backoff_seconds: args.rate_limit_post_reset_max_backoff_seconds,
        post_reset_max_retries: args.rate_limit_post_reset_max_retries,
    };

    loop {
        let current_iteration = iteration + 1;

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

        let mut skip_normal_delay = false;
        let mut count_iteration = true;

        tokio::select! {
            _ = signal::ctrl_c() => {
                cancel_token_clone.cancel();
                println!();
                output::warn(&format!("Ralph loop interrupted after {iteration} iterations"));
                let duration = start_time.elapsed();
                output::log(&format!("Total runtime: {}", output::format_duration(duration)));
                return Ok(());
            }
            result = iteration::run(current_iteration, &ctx, &cancel_token) => {
                match result {
                    Ok(IterationResult::Continue) => {
                        consecutive_failures = 0;
                        rate_limit_state.clear();
                    }
                    Ok(IterationResult::Complete) => {
                        rate_limit_state.clear();
                        println!();
                        output::separator();
                        output::success("Completion marker found! Ralph loop finished.");
                        output::separator();
                        let duration = start_time.elapsed();
                        output::log(&format!("Total iterations: {current_iteration}"));
                        output::log(&format!("Total runtime: {}", output::format_duration(duration)));
                        output::log(&format!("Logs saved to: {}", logs_dir.display()));
                        if let Some(ref url) = args.webhook {
                            webhook::send_webhook(url, EventType::SessionComplete, &format!("Session complete after {current_iteration} iterations"));
                        }
                        return Ok(());
                    }
                    Ok(IterationResult::RateLimit(info)) => {
                        count_iteration = false;
                        skip_normal_delay = true;
                        handle_rate_limit(
                            &info,
                            &mut rate_limit_state,
                            &rate_limit_policy,
                            current_iteration,
                            start_time,
                            &logs_dir,
                            args.webhook.as_deref(),
                        ).await?;
                    }
                    Ok(IterationResult::LoopDetected) => {
                        rate_limit_state.clear();
                        output::warn("Loop detection: Agent appears blocked");
                        handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                        handle_failure(&mut consecutive_failures, current_iteration, start_time, &logs_dir, args.webhook.as_deref())?;
                    }
                    Ok(IterationResult::Failed) => {
                        rate_limit_state.clear();
                        handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                        handle_failure(&mut consecutive_failures, current_iteration, start_time, &logs_dir, args.webhook.as_deref())?;
                    }
                    Err(e) => {
                        rate_limit_state.clear();
                        output::error(&format!("Iteration error: {e:#}"));
                        handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                        handle_failure(&mut consecutive_failures, current_iteration, start_time, &logs_dir, args.webhook.as_deref())?;
                    }
                }
            }
        }

        if count_iteration {
            iteration += 1;
        }

        if args.max_iterations > 0 && iteration >= args.max_iterations {
            println!();
            output::warn(&format!("Max iterations ({}) reached", args.max_iterations));
            let duration = start_time.elapsed();
            output::log(&format!(
                "Total runtime: {}",
                output::format_duration(duration)
            ));
            output::log(&format!("Logs saved to: {}", logs_dir.display()));
            return Ok(());
        }

        if skip_normal_delay {
            println!();
            continue;
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
        output::log(&format!(
            "Total runtime: {}",
            output::format_duration(duration)
        ));
        output::log(&format!("Logs saved to: {}", logs_dir.display()));
        if let Some(url) = webhook_url {
            webhook::send_webhook(
                url,
                EventType::SessionFailed,
                &format!(
                    "Session failed after {iteration} iterations: too many consecutive failures"
                ),
            );
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

async fn handle_rate_limit(
    info: &RateLimitInfo,
    state: &mut rate_limit::RateLimitState,
    policy: &RetryPolicy,
    iteration: u32,
    start_time: std::time::Instant,
    logs_dir: &std::path::Path,
    webhook_url: Option<&str>,
) -> Result<()> {
    output::error(&format!("Rate limit detected: {}", info.raw_message));

    match rate_limit::plan_retry(info, state, policy) {
        RetryDecision::Wait(wait) => {
            match wait.reason {
                RetryReason::ParsedReset => output::warn(&format!(
                    "Parsed reset time. Waiting {} until {} before retry...",
                    output::format_duration(wait.duration),
                    format_retry_deadline(wait.retry_at)
                )),
                RetryReason::ExistingReset => output::warn(&format!(
                    "Rate limit reset already known. Waiting {} until {} before retry...",
                    output::format_duration(wait.duration),
                    format_retry_deadline(wait.retry_at)
                )),
                RetryReason::Fallback => output::warn(&format!(
                    "No reset time parsed. Waiting {} before retry...",
                    output::format_duration(wait.duration)
                )),
                RetryReason::PostResetBackoff => output::warn(&format!(
                    "Still rate-limited after the expected reset. Backing off {} before retry (post-reset failure #{})...",
                    output::format_duration(wait.duration),
                    wait.post_reset_failures
                )),
            }
            sleep(wait.duration).await;
            Ok(())
        }
        RetryDecision::Abort(abort) => {
            println!();
            output::separator();
            output::error("Rate limit persisted after the expected reset window.");
            if let Some(active_reset_at) = abort.active_reset_at {
                output::error(&format!(
                    "Original reset target: {}",
                    format_retry_deadline(
                        active_reset_at
                            + chrono::Duration::seconds(policy.reset_buffer_seconds as i64)
                    )
                ));
            }
            output::error(&format!(
                "Post-reset retries attempted: {}",
                abort.post_reset_failures
            ));
            output::error(&format!("Last rate-limit message: {}", abort.last_message));
            output::separator();
            let duration = start_time.elapsed();
            output::log(&format!("Total iterations: {iteration}"));
            output::log(&format!(
                "Total runtime: {}",
                output::format_duration(duration)
            ));
            output::log(&format!("Logs saved to: {}", logs_dir.display()));
            if let Some(url) = webhook_url {
                webhook::send_webhook(
                    url,
                    EventType::SessionFailed,
                    &format!(
                        "Session failed after {iteration} iterations: persistent post-reset rate limiting"
                    ),
                );
            }
            bail!("Persistent post-reset rate limiting");
        }
    }
}

fn format_retry_deadline(retry_at: chrono::DateTime<Utc>) -> String {
    retry_at
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %Z")
        .to_string()
}
