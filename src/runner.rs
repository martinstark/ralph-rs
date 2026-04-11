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
    retry, subprocess,
    webhook::{self, EventType},
};
use anyhow::{bail, Context, Result};
use chrono::{Local, Utc};
use tokio::signal;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const INTERRUPTED_EXIT_CODE: i32 = 130;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterruptAction {
    RequestShutdown,
    ForceExit,
}

struct ShutdownMonitor {
    token: CancellationToken,
    task: JoinHandle<()>,
}

impl ShutdownMonitor {
    fn install() -> Self {
        let token = CancellationToken::new();
        let signal_token = token.clone();
        let task = tokio::spawn(async move {
            monitor_for_interrupts(signal_token).await;
        });
        Self { token, task }
    }

    fn token(&self) -> &CancellationToken {
        &self.token
    }
}

impl Drop for ShutdownMonitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn run(args: Args) -> Result<RunOutcome> {
    let shutdown = ShutdownMonitor::install();
    let shutdown_token = shutdown.token();
    let start_time = std::time::Instant::now();

    if !args.prd.exists() {
        output::error(&format!("PRD file not found: {}", args.prd.display()));
        output::log("Run 'ralph --init' to create a template, or specify path with -p");
        bail!("PRD file not found");
    }

    let prd = prd::Prd::load(&args.prd)?;
    let completion_marker = config::resolve_completion_marker(args.completion_marker.as_deref())?;
    let project_dir = args
        .prd
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let progress_path = project_dir.join("progress.txt");
    let ralph_dir = project_dir.join(".ralph");
    let logs_dir = ralph_dir.join("logs");

    if args.dry_run {
        match dry_run::run(&args, &prd, shutdown_token).await {
            Ok(true) => {
                log_interrupted(0, start_time, None);
                return Ok(RunOutcome::Interrupted);
            }
            Ok(false) => return Ok(RunOutcome::Completed),
            Err(error) if shutdown_token.is_cancelled() => {
                log_shutdown_cleanup_error("Dry-run shutdown did not finish cleanly", &error);
                log_interrupted(0, start_time, None);
                return Ok(RunOutcome::Interrupted);
            }
            Err(error) => return Err(error),
        }
    }

    std::fs::create_dir_all(&logs_dir).context("Failed to create .ralph/logs directory")?;

    if !progress_path.exists() {
        std::fs::write(
            &progress_path,
            "# Ralph Progress Log\n\nAppend-only log of session activity.\n\n---\n\n",
        )?;
    }

    if !args.skip_init {
        match init::run_init_phase(&prd, &args.prd, &progress_path, shutdown_token).await {
            Ok(true) => {
                log_interrupted(0, start_time, Some(&logs_dir));
                return Ok(RunOutcome::Interrupted);
            }
            Ok(false) => {}
            Err(error) if shutdown_token.is_cancelled() => {
                log_shutdown_cleanup_error(
                    "Initialization shutdown did not finish cleanly",
                    &error,
                );
                log_interrupted(0, start_time, Some(&logs_dir));
                return Ok(RunOutcome::Interrupted);
            }
            Err(error) => return Err(error),
        }
    }

    if shutdown_token.is_cancelled() {
        log_interrupted(0, start_time, Some(&logs_dir));
        return Ok(RunOutcome::Interrupted);
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
    output::log(&format!(
        "Rate-limit handling: {}",
        if args.exit_on_rate_limit {
            "exit immediately"
        } else {
            "retry with backoff"
        }
    ));
    println!();

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
        if shutdown_token.is_cancelled() {
            log_interrupted(iteration, start_time, Some(&logs_dir));
            return Ok(RunOutcome::Interrupted);
        }

        let current_iteration = iteration + 1;
        let current_prd = prd::Prd::load(&args.prd)?;
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

        match iteration::run(current_iteration, &ctx, shutdown_token).await {
            Ok(IterationResult::Cancelled) => {
                log_interrupted(iteration, start_time, Some(&logs_dir));
                return Ok(RunOutcome::Interrupted);
            }
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
                output::log(&format!(
                    "Total runtime: {}",
                    output::format_duration(duration)
                ));
                output::log(&format!("Logs saved to: {}", logs_dir.display()));
                if let Some(ref url) = args.webhook {
                    webhook::send_webhook(
                        url,
                        EventType::SessionComplete,
                        &format!("Session complete after {current_iteration} iterations"),
                    );
                }
                return Ok(RunOutcome::Completed);
            }
            Ok(IterationResult::RateLimit(info)) => {
                count_iteration = false;
                skip_normal_delay = true;
                if handle_rate_limit(
                    &info,
                    &mut rate_limit_state,
                    &rate_limit_policy,
                    args.exit_on_rate_limit,
                    current_iteration,
                    start_time,
                    &logs_dir,
                    args.webhook.as_deref(),
                    shutdown_token,
                )
                .await?
                {
                    log_interrupted(iteration, start_time, Some(&logs_dir));
                    return Ok(RunOutcome::Interrupted);
                }
            }
            Ok(IterationResult::LoopDetected) => {
                rate_limit_state.clear();
                output::warn("Loop detection: Agent appears blocked");
                handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                handle_failure(
                    &mut consecutive_failures,
                    current_iteration,
                    start_time,
                    &logs_dir,
                    args.webhook.as_deref(),
                )?;
            }
            Ok(IterationResult::Failed) => {
                rate_limit_state.clear();
                handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                handle_failure(
                    &mut consecutive_failures,
                    current_iteration,
                    start_time,
                    &logs_dir,
                    args.webhook.as_deref(),
                )?;
            }
            Err(e) => {
                rate_limit_state.clear();
                if shutdown_token.is_cancelled() {
                    log_shutdown_cleanup_error("Interrupt cleanup did not finish cleanly", &e);
                    log_interrupted(iteration, start_time, Some(&logs_dir));
                    return Ok(RunOutcome::Interrupted);
                }
                output::error(&format!("Iteration error: {e:#}"));
                handle_iteration_error(&mut error_tracker, &args.prd, &current_prd)?;
                handle_failure(
                    &mut consecutive_failures,
                    current_iteration,
                    start_time,
                    &logs_dir,
                    args.webhook.as_deref(),
                )?;
            }
        }

        if count_iteration {
            iteration += 1;
        }

        println!();
        if args.max_iterations > 0 && iteration >= args.max_iterations {
            output::warn(&format!("Max iterations ({}) reached", args.max_iterations));
            let duration = start_time.elapsed();
            output::log(&format!(
                "Total runtime: {}",
                output::format_duration(duration)
            ));
            output::log(&format!("Logs saved to: {}", logs_dir.display()));
            return Ok(RunOutcome::Completed);
        }

        println!();
        if skip_normal_delay {
            continue;
        }

        output::dim(&format!("Waiting {}s before next iteration...", args.delay));
        if sleep_with_shutdown(Duration::from_secs(args.delay), shutdown_token).await {
            log_interrupted(iteration, start_time, Some(&logs_dir));
            return Ok(RunOutcome::Interrupted);
        }
        println!();
    }
}

async fn monitor_for_interrupts(shutdown_token: CancellationToken) {
    let mut interrupt_count = 0;
    while signal::ctrl_c().await.is_ok() {
        match interrupt_action_for_count(interrupt_count) {
            InterruptAction::RequestShutdown => {
                shutdown_token.cancel();
                println!();
                output::warn(
                    "Ctrl+C received. Waiting for cleanup to finish. Press Ctrl+C again to force exit.",
                );
            }
            InterruptAction::ForceExit => {
                println!();
                output::error("Second Ctrl+C received. Forcing immediate exit.");
                if let Err(error) = subprocess::force_terminate_tracked_processes() {
                    output::warn(&format!(
                        "Failed to force-kill active subprocesses before exit: {error:#}"
                    ));
                }
                std::process::exit(INTERRUPTED_EXIT_CODE);
            }
        }
        interrupt_count += 1;
    }
}

fn interrupt_action_for_count(interrupt_count: usize) -> InterruptAction {
    if interrupt_count == 0 {
        InterruptAction::RequestShutdown
    } else {
        InterruptAction::ForceExit
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
    exit_on_rate_limit: bool,
    iteration: u32,
    start_time: std::time::Instant,
    logs_dir: &std::path::Path,
    webhook_url: Option<&str>,
    shutdown_token: &CancellationToken,
) -> Result<bool> {
    output::error(&format!("Rate limit detected: {}", info.raw_message));

    if exit_on_rate_limit {
        println!();
        output::separator();
        output::error("Rate limit retries are disabled for this run.");
        output::error(&format!("Last rate-limit message: {}", info.raw_message));
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
                    "Session failed after {iteration} iterations: rate limit detected and retries disabled"
                ),
            );
        }
        bail!("Rate limit detected and retries disabled");
    }

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
            Ok(sleep_with_shutdown(wait.duration, shutdown_token).await)
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

async fn sleep_with_shutdown(duration: Duration, shutdown_token: &CancellationToken) -> bool {
    tokio::select! {
        _ = shutdown_token.cancelled() => true,
        _ = sleep(duration) => false,
    }
}

fn log_interrupted(
    iteration: u32,
    start_time: std::time::Instant,
    logs_dir: Option<&std::path::Path>,
) {
    println!();
    output::warn(&format!(
        "Ralph interrupted after {iteration} completed iterations"
    ));
    let duration = start_time.elapsed();
    output::log(&format!(
        "Total runtime: {}",
        output::format_duration(duration)
    ));
    if let Some(logs_dir) = logs_dir {
        output::log(&format!("Logs saved to: {}", logs_dir.display()));
    }
}

fn log_shutdown_cleanup_error(context: &str, error: &anyhow::Error) {
    output::warn(&format!("{context}: {error:#}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate_limit::{RateLimitSource, RetryPolicy};
    use chrono::Utc;
    use std::path::Path;

    #[test]
    fn interrupt_action_requests_shutdown_first() {
        assert_eq!(
            interrupt_action_for_count(0),
            InterruptAction::RequestShutdown
        );
    }

    #[test]
    fn interrupt_action_forces_exit_after_first_signal() {
        assert_eq!(interrupt_action_for_count(1), InterruptAction::ForceExit);
        assert_eq!(interrupt_action_for_count(2), InterruptAction::ForceExit);
    }

    #[tokio::test]
    async fn sleep_with_shutdown_returns_true_when_cancelled() {
        let shutdown_token = CancellationToken::new();
        shutdown_token.cancel();

        assert!(sleep_with_shutdown(Duration::from_millis(10), &shutdown_token).await);
    }

    #[tokio::test]
    async fn sleep_with_shutdown_returns_false_after_waiting() {
        let shutdown_token = CancellationToken::new();

        assert!(!sleep_with_shutdown(Duration::from_millis(1), &shutdown_token).await);
    }


    #[tokio::test]
    async fn handle_rate_limit_aborts_when_exit_on_rate_limit_enabled() {
        let shutdown_token = CancellationToken::new();
        let info = RateLimitInfo {
            reset_at: None,
            observed_at: Utc::now(),
            raw_message: "Too many requests".to_string(),
            source: RateLimitSource::Generic,
        };
        let mut state = rate_limit::RateLimitState::default();

        let error = handle_rate_limit(
            &info,
            &mut state,
            &RetryPolicy::default(),
            true,
            1,
            std::time::Instant::now(),
            Path::new("."),
            None,
            &shutdown_token,
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Rate limit detected and retries disabled"
        );
    }

    #[tokio::test]
    async fn handle_rate_limit_waits_when_retry_enabled() {
        let shutdown_token = CancellationToken::new();
        let info = RateLimitInfo {
            reset_at: None,
            observed_at: Utc::now(),
            raw_message: "Too many requests".to_string(),
            source: RateLimitSource::Generic,
        };
        let mut state = rate_limit::RateLimitState::default();
        let policy = RetryPolicy {
            fallback_seconds: 0,
            ..RetryPolicy::default()
        };

        let interrupted = handle_rate_limit(
            &info,
            &mut state,
            &policy,
            false,
            1,
            std::time::Instant::now(),
            Path::new("."),
            None,
            &shutdown_token,
        )
        .await
        .unwrap();

        assert!(!interrupted);
    }
}
