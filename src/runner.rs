use crate::{
    claude::{self, ClaudeArgs},
    config::Args,
    git, init, output, prd, prompt, validation,
};
use anyhow::{bail, Context, Result};
use chrono::Local;
use std::process::Command;
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
        return run_dry_run(&args, &prd);
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
    println!();

    let start_time = std::time::Instant::now();
    let mut iteration: u32 = 0;
    let mut consecutive_failures: u32 = 0;

    loop {
        iteration += 1;

        let cancel_token = CancellationToken::new();
        let cancel_token_clone = cancel_token.clone();

        let ctx = IterationContext {
            args: &args,
            prd: &prd,
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
                        return Ok(());
                    }
                    Ok(IterationResult::RateLimit) => {
                        output::error("Rate limit detected. Waiting 60s before retry...");
                        sleep(Duration::from_secs(60)).await;
                    }
                    Ok(IterationResult::LoopDetected) => {
                        output::warn("Loop detection: Agent appears blocked");
                        handle_failure(&mut consecutive_failures, iteration, start_time, &logs_dir)?;
                    }
                    Ok(IterationResult::Failed) => {
                        handle_failure(&mut consecutive_failures, iteration, start_time, &logs_dir)?;
                    }
                    Err(e) => {
                        output::error(&format!("Iteration error: {e:#}"));
                        handle_failure(&mut consecutive_failures, iteration, start_time, &logs_dir)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IterationResult {
    Continue,
    Complete,
    RateLimit,
    LoopDetected,
    Failed,
}

pub(crate) struct OutputAnalysisContext<'a> {
    pub success: bool,
    pub completion_marker: &'a str,
}

#[must_use]
pub(crate) fn analyze_iteration_output(output: &str, ctx: &OutputAnalysisContext<'_>) -> IterationResult {
    if !ctx.success && detect_rate_limit(output) {
        return IterationResult::RateLimit;
    }
    if detect_loop_pattern(output) {
        return IterationResult::LoopDetected;
    }
    if output.contains(ctx.completion_marker) {
        return IterationResult::Complete;
    }
    if ctx.success {
        IterationResult::Continue
    } else {
        IterationResult::Failed
    }
}

/// Handles failure by incrementing counter and checking if max failures reached.
/// Returns Err if too many consecutive failures, Ok(()) otherwise.
fn handle_failure(
    consecutive_failures: &mut u32,
    iteration: u32,
    start_time: std::time::Instant,
    logs_dir: &std::path::Path,
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
        bail!("Too many consecutive failures");
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

#[must_use]
pub(crate) fn detect_loop_pattern(output: &str) -> bool {
    // Only check first 500 chars - stuck messages appear at start
    let check_region: String = output.chars().take(500).collect();
    let lower = check_region.to_lowercase();

    let patterns = [
        "i cannot proceed",
        "i'm unable to continue",
        "i don't have access to",
        "cannot complete this task",
    ];

    patterns.iter().any(|p| lower.contains(p))
}

#[must_use]
pub(crate) fn detect_rate_limit(output: &str) -> bool {
    // Check last 1000 chars where error messages appear
    let tail = output
        .char_indices()
        .rev()
        .nth(999)
        .map_or(output, |(i, _)| &output[i..]);
    let lower = tail.to_lowercase();

    lower.contains("rate limit") || lower.contains("too many requests")
}

fn run_dry_run(args: &Args, prd: &prd::Prd) -> Result<()> {
    output::section("Dry Run Mode");

    output::header("PRD Summary");
    output::log(&format!("Project: {}", prd.project.name));
    output::log(&format!("PRD file: {}", args.prd.display()));
    println!();

    let counts = prd.status_counts();
    let total = prd.features.len();
    output::header("Feature Status");
    output::log(&format!("Total features: {total}"));
    output::log(&format!(
        "  Pending:     {}",
        counts.pending
    ));
    output::log(&format!(
        "  In-progress: {}",
        counts.in_progress
    ));
    output::log(&format!(
        "  Complete:    {}",
        counts.complete
    ));
    output::log(&format!(
        "  Blocked:     {}",
        counts.blocked
    ));
    println!();

    output::header("Git Status");
    if let Some(status) = git::get_git_status() {
        output::log(&format!("Branch: {}", status.branch));
        output::log(&format!("Uncommitted changes: {}", status.uncommitted_changes));
        if status.uncommitted_changes > 0 {
            output::dim("  (Uncommitted changes are informational only)");
        }
    } else {
        output::warn("Not a git repository");
    }
    println!();

    output::header("Verification Commands");
    let mut all_passed = true;
    for cmd in &prd.verification.commands {
        let result = Command::new("sh")
            .args(["-c", &cmd.command])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                output::success(&format!("{}: PASS", cmd.name));
            }
            Ok(_) => {
                output::error(&format!("{}: FAIL", cmd.name));
                all_passed = false;
            }
            Err(e) => {
                output::error(&format!("{}: ERROR ({})", cmd.name, e));
                all_passed = false;
            }
        }
    }
    println!();

    output::separator();
    if all_passed {
        output::success("Dry run complete - all verifications passed");
    } else {
        output::warn("Dry run complete - some verifications failed");
    }
    output::separator();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod detect_loop_pattern_tests {
        use super::*;

        #[test]
        fn detects_cannot_proceed() {
            assert!(detect_loop_pattern("I cannot proceed with this task"));
        }

        #[test]
        fn detects_unable_to_continue() {
            assert!(detect_loop_pattern("I'm unable to continue without more info"));
        }

        #[test]
        fn detects_no_access() {
            assert!(detect_loop_pattern("I don't have access to those files"));
        }

        #[test]
        fn detects_cannot_complete() {
            assert!(detect_loop_pattern("Cannot complete this task as requested"));
        }

        #[test]
        fn case_insensitive() {
            assert!(detect_loop_pattern("I CANNOT PROCEED with this"));
            assert!(detect_loop_pattern("I'M UNABLE TO CONTINUE"));
        }

        #[test]
        fn returns_false_for_normal_output() {
            assert!(!detect_loop_pattern("Task completed successfully"));
            assert!(!detect_loop_pattern("Working on the feature now"));
        }

        #[test]
        fn only_checks_first_500_chars() {
            let mut output = "x".repeat(600);
            output.push_str("I cannot proceed");
            assert!(!detect_loop_pattern(&output));
        }

        #[test]
        fn detects_within_first_500_chars() {
            let mut output = "x".repeat(400);
            output.push_str("I cannot proceed");
            assert!(detect_loop_pattern(&output));
        }

        #[test]
        fn handles_empty_string() {
            assert!(!detect_loop_pattern(""));
        }
    }

    mod detect_rate_limit_tests {
        use super::*;

        #[test]
        fn detects_rate_limit() {
            assert!(detect_rate_limit("Error: rate limit exceeded"));
        }

        #[test]
        fn detects_too_many_requests() {
            assert!(detect_rate_limit("Too many requests, please wait"));
        }

        #[test]
        fn case_insensitive() {
            assert!(detect_rate_limit("RATE LIMIT hit"));
            assert!(detect_rate_limit("TOO MANY REQUESTS"));
        }

        #[test]
        fn returns_false_for_normal_output() {
            assert!(!detect_rate_limit("Task completed successfully"));
            assert!(!detect_rate_limit("Processing request"));
        }

        #[test]
        fn only_checks_last_1000_chars() {
            let mut output = String::from("rate limit error at start");
            output.push_str(&"x".repeat(1500));
            assert!(!detect_rate_limit(&output));
        }

        #[test]
        fn detects_within_last_1000_chars() {
            let mut output = "x".repeat(500);
            output.push_str("rate limit error");
            assert!(detect_rate_limit(&output));
        }

        #[test]
        fn handles_empty_string() {
            assert!(!detect_rate_limit(""));
        }

        #[test]
        fn handles_short_string() {
            assert!(detect_rate_limit("rate limit"));
            assert!(!detect_rate_limit("ok"));
        }
    }

    mod analyze_iteration_output_tests {
        use super::*;

        fn ctx(success: bool, marker: &str) -> OutputAnalysisContext<'_> {
            OutputAnalysisContext {
                success,
                completion_marker: marker,
            }
        }

        #[test]
        fn returns_rate_limit_on_failure_with_rate_limit() {
            let result = analyze_iteration_output("Error: rate limit", &ctx(false, "DONE"));
            assert_eq!(result, IterationResult::RateLimit);
        }

        #[test]
        fn returns_loop_detected_on_stuck_pattern() {
            let result = analyze_iteration_output("I cannot proceed", &ctx(true, "DONE"));
            assert_eq!(result, IterationResult::LoopDetected);
        }

        #[test]
        fn returns_complete_when_marker_found() {
            let result = analyze_iteration_output("Task DONE successfully", &ctx(true, "DONE"));
            assert_eq!(result, IterationResult::Complete);
        }

        #[test]
        fn returns_continue_on_success_without_marker() {
            let result = analyze_iteration_output("Working on it", &ctx(true, "DONE"));
            assert_eq!(result, IterationResult::Continue);
        }

        #[test]
        fn returns_failed_on_failure_without_rate_limit() {
            let result = analyze_iteration_output("Some error occurred", &ctx(false, "DONE"));
            assert_eq!(result, IterationResult::Failed);
        }

        #[test]
        fn rate_limit_takes_priority_over_loop_detection() {
            let output = "I cannot proceed\nrate limit";
            let result = analyze_iteration_output(output, &ctx(false, "DONE"));
            assert_eq!(result, IterationResult::RateLimit);
        }

        #[test]
        fn loop_detection_takes_priority_over_completion() {
            let output = "I cannot proceed DONE";
            let result = analyze_iteration_output(output, &ctx(true, "DONE"));
            assert_eq!(result, IterationResult::LoopDetected);
        }

        #[test]
        fn completion_marker_exact_match() {
            let result = analyze_iteration_output("<promise>COMPLETE</promise>", &ctx(true, "<promise>COMPLETE</promise>"));
            assert_eq!(result, IterationResult::Complete);
        }

        #[test]
        fn empty_marker_always_matches() {
            // Empty string is contained in any string
            let result = analyze_iteration_output("any output", &ctx(true, ""));
            assert_eq!(result, IterationResult::Complete);
        }
    }

    mod boundary_tests {
        use super::*;

        #[test]
        fn loop_pattern_at_exactly_500_chars() {
            // Pattern starts at char 484, ends within 500
            let mut output = "x".repeat(484);
            output.push_str("I cannot proceed");
            assert!(detect_loop_pattern(&output));
        }

        #[test]
        fn loop_pattern_just_past_500_chars() {
            // Pattern starts at char 485, extends past 500-char window
            let mut output = "x".repeat(485);
            output.push_str("I cannot proceed");
            assert!(!detect_loop_pattern(&output));
        }

        #[test]
        fn rate_limit_at_exactly_1000_chars_from_end() {
            let mut output = "x".repeat(500);
            output.push_str("rate limit");
            output.push_str(&"y".repeat(490)); // total = 500 + 10 + 490 = 1000
            assert!(detect_rate_limit(&output));
        }

        #[test]
        fn rate_limit_just_past_1000_chars_from_end() {
            let mut output = String::from("rate limit");
            output.push_str(&"x".repeat(1001)); // pattern is 1011 chars from end
            assert!(!detect_rate_limit(&output));
        }
    }
}
