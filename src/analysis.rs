#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterationResult {
    Continue,
    Complete,
    RateLimit,
    LoopDetected,
    Failed,
}

pub struct OutputAnalysisContext<'a> {
    pub success: bool,
    pub completion_marker: &'a str,
}

#[must_use]
pub fn analyze_iteration_output(output: &str, ctx: &OutputAnalysisContext<'_>) -> IterationResult {
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

#[must_use]
pub fn detect_loop_pattern(output: &str) -> bool {
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
pub fn detect_rate_limit(output: &str) -> bool {
    // Check last 1000 chars where error messages appear
    let tail = output
        .char_indices()
        .rev()
        .nth(999)
        .map_or(output, |(i, _)| &output[i..]);
    let lower = tail.to_lowercase();

    lower.contains("rate limit") || lower.contains("too many requests")
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
            let result = analyze_iteration_output("any output", &ctx(true, ""));
            assert_eq!(result, IterationResult::Complete);
        }
    }

    mod boundary_tests {
        use super::*;

        #[test]
        fn loop_pattern_at_exactly_500_chars() {
            let mut output = "x".repeat(484);
            output.push_str("I cannot proceed");
            assert!(detect_loop_pattern(&output));
        }

        #[test]
        fn loop_pattern_just_past_500_chars() {
            let mut output = "x".repeat(485);
            output.push_str("I cannot proceed");
            assert!(!detect_loop_pattern(&output));
        }

        #[test]
        fn rate_limit_at_exactly_1000_chars_from_end() {
            let mut output = "x".repeat(500);
            output.push_str("rate limit");
            output.push_str(&"y".repeat(490));
            assert!(detect_rate_limit(&output));
        }

        #[test]
        fn rate_limit_just_past_1000_chars_from_end() {
            let mut output = String::from("rate limit");
            output.push_str(&"x".repeat(1001));
            assert!(!detect_rate_limit(&output));
        }
    }
}
