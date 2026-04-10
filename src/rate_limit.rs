use chrono::{
    DateTime, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc,
};
use chrono_tz::Tz;
use std::time::Duration;

pub const DEFAULT_FALLBACK_SECONDS: u64 = 60;
pub const DEFAULT_RESET_BUFFER_SECONDS: u64 = 60;
pub const DEFAULT_POST_RESET_MAX_BACKOFF_SECONDS: u64 = 1800;
pub const DEFAULT_POST_RESET_MAX_RETRIES: u32 = 4;

const RATE_LIMIT_TAIL_CHARS: usize = 1000;
const NONEXISTENT_TIME_RESOLUTION_MINUTES: i64 = 180;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitSource {
    ParsedReset,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitInfo {
    pub reset_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    pub raw_message: String,
    pub source: RateLimitSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RateLimitState {
    pub active_reset_at: Option<DateTime<Utc>>,
    pub post_reset_failures: u32,
}

impl RateLimitState {
    pub fn clear(&mut self) {
        self.active_reset_at = None;
        self.post_reset_failures = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    pub fallback_seconds: u64,
    pub reset_buffer_seconds: u64,
    pub post_reset_max_backoff_seconds: u64,
    pub post_reset_max_retries: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            fallback_seconds: DEFAULT_FALLBACK_SECONDS,
            reset_buffer_seconds: DEFAULT_RESET_BUFFER_SECONDS,
            post_reset_max_backoff_seconds: DEFAULT_POST_RESET_MAX_BACKOFF_SECONDS,
            post_reset_max_retries: DEFAULT_POST_RESET_MAX_RETRIES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryReason {
    ParsedReset,
    ExistingReset,
    Fallback,
    PostResetBackoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryWait {
    pub duration: Duration,
    pub retry_at: DateTime<Utc>,
    pub reason: RetryReason,
    pub active_reset_at: Option<DateTime<Utc>>,
    pub post_reset_failures: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryAbort {
    pub active_reset_at: Option<DateTime<Utc>>,
    pub post_reset_failures: u32,
    pub last_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    Wait(RetryWait),
    Abort(RetryAbort),
}

#[must_use]
pub fn detect_rate_limit(output: &str) -> bool {
    let lower = relevant_tail(output).to_lowercase();
    rate_limit_pattern_found(&lower)
}

#[must_use]
pub fn parse_rate_limit_info(output: &str, observed_at: DateTime<Utc>) -> Option<RateLimitInfo> {
    let tail = relevant_tail(output);
    if !detect_rate_limit(tail) {
        return None;
    }

    let raw_message = extract_raw_message(tail);
    let reset_at = parse_reset_at(tail, observed_at);
    let source = if reset_at.is_some() {
        RateLimitSource::ParsedReset
    } else {
        RateLimitSource::Generic
    };

    Some(RateLimitInfo {
        reset_at,
        observed_at,
        raw_message,
        source,
    })
}

pub fn plan_retry(
    info: &RateLimitInfo,
    state: &mut RateLimitState,
    policy: &RetryPolicy,
) -> RetryDecision {
    let buffer = ChronoDuration::seconds(policy.reset_buffer_seconds as i64);

    if let Some(active_reset_at) = state.active_reset_at {
        if info.observed_at >= active_reset_at + buffer {
            state.post_reset_failures += 1;
            if policy.post_reset_max_retries > 0
                && state.post_reset_failures >= policy.post_reset_max_retries
            {
                return RetryDecision::Abort(RetryAbort {
                    active_reset_at: state.active_reset_at,
                    post_reset_failures: state.post_reset_failures,
                    last_message: info.raw_message.clone(),
                });
            }

            let duration = post_reset_backoff(
                state.post_reset_failures,
                policy.post_reset_max_backoff_seconds,
            );
            let retry_at = info.observed_at
                + ChronoDuration::from_std(duration)
                    .expect("post-reset backoff duration should fit chrono");

            return RetryDecision::Wait(RetryWait {
                duration,
                retry_at,
                reason: RetryReason::PostResetBackoff,
                active_reset_at: state.active_reset_at,
                post_reset_failures: state.post_reset_failures,
            });
        }
    }

    if let Some(parsed_reset_at) = info.reset_at {
        let effective_reset_at = match state.active_reset_at {
            Some(active_reset_at) if active_reset_at > parsed_reset_at => active_reset_at,
            _ => {
                state.active_reset_at = Some(parsed_reset_at);
                parsed_reset_at
            }
        };
        state.post_reset_failures = 0;

        let retry_at = effective_reset_at + buffer;
        return RetryDecision::Wait(RetryWait {
            duration: duration_until(info.observed_at, retry_at),
            retry_at,
            reason: RetryReason::ParsedReset,
            active_reset_at: state.active_reset_at,
            post_reset_failures: state.post_reset_failures,
        });
    }

    if let Some(active_reset_at) = state.active_reset_at {
        let retry_at = active_reset_at + buffer;
        if retry_at > info.observed_at {
            return RetryDecision::Wait(RetryWait {
                duration: duration_until(info.observed_at, retry_at),
                retry_at,
                reason: RetryReason::ExistingReset,
                active_reset_at: state.active_reset_at,
                post_reset_failures: state.post_reset_failures,
            });
        }
    }

    let duration = Duration::from_secs(policy.fallback_seconds);
    let retry_at = info.observed_at
        + ChronoDuration::seconds(policy.fallback_seconds.try_into().unwrap_or(i64::MAX));
    RetryDecision::Wait(RetryWait {
        duration,
        retry_at,
        reason: RetryReason::Fallback,
        active_reset_at: state.active_reset_at,
        post_reset_failures: state.post_reset_failures,
    })
}

#[must_use]
pub fn post_reset_backoff(failure_count: u32, max_backoff_seconds: u64) -> Duration {
    let scheduled_seconds = match failure_count {
        0 => 0,
        1 => 5 * 60,
        2 => 15 * 60,
        _ => 30 * 60,
    };
    Duration::from_secs(scheduled_seconds.min(max_backoff_seconds))
}

fn relevant_tail(output: &str) -> &str {
    output
        .char_indices()
        .rev()
        .nth(RATE_LIMIT_TAIL_CHARS.saturating_sub(1))
        .map_or(output, |(idx, _)| &output[idx..])
}

fn rate_limit_pattern_found(lower: &str) -> bool {
    lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("you've hit your limit")
        || lower.contains("you have hit your limit")
        || lower.contains("hit your limit")
}

fn extract_raw_message(tail: &str) -> String {
    tail.lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && (rate_limit_pattern_found(&line.to_lowercase())
                    || line.to_lowercase().contains("resets "))
        })
        .unwrap_or_else(|| tail.trim())
        .to_string()
}

fn parse_reset_at(text: &str, observed_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (time_str, timezone_str) = extract_reset_clause(text)?;
    let (hour, minute) = parse_reset_time(time_str)?;
    let timezone = timezone_str.parse::<Tz>().ok()?;
    resolve_reset_timestamp(observed_at, timezone, hour, minute)
}

fn extract_reset_clause(text: &str) -> Option<(&str, &str)> {
    let lower = text.to_lowercase();
    let reset_index = lower.rfind("resets ")?;
    let remainder = text.get(reset_index + "resets ".len()..)?.trim_start();
    let timezone_start = remainder.find('(')?;
    let time_str = remainder[..timezone_start].trim();
    let timezone_remainder = remainder.get(timezone_start + 1..)?;
    let timezone_end = timezone_remainder.find(')')?;
    let timezone_str = timezone_remainder[..timezone_end].trim();

    if time_str.is_empty() || timezone_str.is_empty() {
        return None;
    }

    Some((time_str, timezone_str))
}

fn parse_reset_time(input: &str) -> Option<(u32, u32)> {
    let lower = input.trim().to_ascii_lowercase();
    let (clock, is_pm) = if let Some(clock) = lower.strip_suffix("am") {
        (clock.trim(), false)
    } else if let Some(clock) = lower.strip_suffix("pm") {
        (clock.trim(), true)
    } else {
        return None;
    };

    let (hour, minute) = if let Some((hour, minute)) = clock.split_once(':') {
        (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?)
    } else {
        (clock.parse::<u32>().ok()?, 0)
    };

    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }

    let hour = match (hour, is_pm) {
        (12, false) => 0,
        (12, true) => 12,
        (h, true) => h + 12,
        (h, false) => h,
    };

    Some((hour, minute))
}

fn resolve_reset_timestamp(
    observed_at: DateTime<Utc>,
    timezone: Tz,
    hour: u32,
    minute: u32,
) -> Option<DateTime<Utc>> {
    let observed_local = observed_at.with_timezone(&timezone);
    let same_day = resolve_local_datetime(timezone, observed_local.date_naive(), hour, minute)?;
    let reset_local = if same_day > observed_local {
        same_day
    } else {
        let next_day = observed_local.date_naive().succ_opt()?;
        resolve_local_datetime(timezone, next_day, hour, minute)?
    };

    Some(reset_local.with_timezone(&Utc))
}

fn resolve_local_datetime(
    timezone: Tz,
    date: NaiveDate,
    hour: u32,
    minute: u32,
) -> Option<DateTime<Tz>> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    match timezone.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt),
        LocalResult::Ambiguous(earliest, _) => Some(earliest),
        LocalResult::None => resolve_nonexistent_local_datetime(timezone, naive),
    }
}

fn resolve_nonexistent_local_datetime(timezone: Tz, naive: NaiveDateTime) -> Option<DateTime<Tz>> {
    let mut probe = naive;
    for _ in 0..NONEXISTENT_TIME_RESOLUTION_MINUTES {
        probe += ChronoDuration::minutes(1);
        match timezone.from_local_datetime(&probe) {
            LocalResult::Single(dt) => return Some(dt),
            LocalResult::Ambiguous(earliest, _) => return Some(earliest),
            LocalResult::None => {}
        }
    }
    None
}

fn duration_until(from: DateTime<Utc>, to: DateTime<Utc>) -> Duration {
    if to <= from {
        Duration::from_secs(0)
    } else {
        (to - from)
            .to_std()
            .expect("future retry deadline should convert to std::time::Duration")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn observed_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
    }

    mod detect_rate_limit_tests {
        use super::*;

        #[test]
        fn detects_rate_limit_keywords() {
            assert!(detect_rate_limit("Error: rate limit exceeded"));
            assert!(detect_rate_limit("Too many requests, please wait"));
            assert!(detect_rate_limit(
                "You've hit your limit · resets 2am (Europe/Stockholm)"
            ));
        }

        #[test]
        fn only_checks_the_tail() {
            let mut output = String::from("rate limit at start");
            output.push_str(&"x".repeat(1500));
            assert!(!detect_rate_limit(&output));
        }
    }

    mod parse_rate_limit_info_tests {
        use super::*;

        #[test]
        fn parses_stockholm_reset_time() {
            let info = parse_rate_limit_info(
                "You've hit your limit · resets 2am (Europe/Stockholm)",
                observed_at(2026, 4, 9, 23, 15),
            )
            .unwrap();

            assert_eq!(info.source, RateLimitSource::ParsedReset);
            assert_eq!(info.reset_at, Some(observed_at(2026, 4, 10, 0, 0)));
            assert_eq!(
                info.reset_at
                    .unwrap()
                    .with_timezone(&chrono_tz::Europe::Stockholm),
                chrono_tz::Europe::Stockholm
                    .with_ymd_and_hms(2026, 4, 10, 2, 0, 0)
                    .single()
                    .unwrap()
            );
        }

        #[test]
        fn parses_los_angeles_reset_time_with_minutes() {
            let info = parse_rate_limit_info(
                "You've hit your limit - resets 2:30pm (America/Los_Angeles)",
                observed_at(2026, 4, 10, 18, 0),
            )
            .unwrap();

            assert_eq!(info.source, RateLimitSource::ParsedReset);
            assert_eq!(
                info.reset_at
                    .unwrap()
                    .with_timezone(&chrono_tz::America::Los_Angeles),
                chrono_tz::America::Los_Angeles
                    .with_ymd_and_hms(2026, 4, 10, 14, 30, 0)
                    .single()
                    .unwrap()
            );
        }

        #[test]
        fn rolls_forward_to_next_local_day_when_reset_time_has_passed() {
            let info = parse_rate_limit_info(
                "You've hit your limit. resets 2am (Europe/Stockholm)",
                observed_at(2026, 4, 9, 22, 10),
            )
            .unwrap();

            assert_eq!(
                info.reset_at
                    .unwrap()
                    .with_timezone(&chrono_tz::Europe::Stockholm),
                chrono_tz::Europe::Stockholm
                    .with_ymd_and_hms(2026, 4, 10, 2, 0, 0)
                    .single()
                    .unwrap()
            );

            let rolled = parse_rate_limit_info(
                "You've hit your limit. resets 2am (Europe/Stockholm)",
                observed_at(2026, 4, 10, 1, 10),
            )
            .unwrap();

            assert_eq!(
                rolled
                    .reset_at
                    .unwrap()
                    .with_timezone(&chrono_tz::Europe::Stockholm),
                chrono_tz::Europe::Stockholm
                    .with_ymd_and_hms(2026, 4, 11, 2, 0, 0)
                    .single()
                    .unwrap()
            );
        }

        #[test]
        fn rejects_malformed_timezone() {
            let info = parse_rate_limit_info(
                "You've hit your limit · resets 2am (Not/AZone)",
                observed_at(2026, 4, 10, 0, 15),
            )
            .unwrap();

            assert_eq!(info.source, RateLimitSource::Generic);
            assert!(info.reset_at.is_none());
        }

        #[test]
        fn rejects_malformed_time() {
            let info = parse_rate_limit_info(
                "You've hit your limit · resets soon (Europe/Stockholm)",
                observed_at(2026, 4, 10, 0, 15),
            )
            .unwrap();

            assert_eq!(info.source, RateLimitSource::Generic);
            assert!(info.reset_at.is_none());
        }

        #[test]
        fn handles_message_without_reset_clause() {
            let info = parse_rate_limit_info("Too many requests", observed_at(2026, 4, 10, 0, 15))
                .unwrap();

            assert_eq!(info.source, RateLimitSource::Generic);
            assert!(info.reset_at.is_none());
            assert_eq!(info.raw_message, "Too many requests");
        }

        #[test]
        fn handles_daylight_saving_gap_by_rolling_forward_to_next_valid_time() {
            let info = parse_rate_limit_info(
                "You've hit your limit · resets 2:30am (Europe/Stockholm)",
                observed_at(2026, 3, 28, 23, 0),
            )
            .unwrap();

            assert_eq!(
                info.reset_at
                    .unwrap()
                    .with_timezone(&chrono_tz::Europe::Stockholm),
                chrono_tz::Europe::Stockholm
                    .with_ymd_and_hms(2026, 3, 29, 3, 0, 0)
                    .single()
                    .unwrap()
            );
        }
    }

    mod retry_policy_tests {
        use super::*;

        fn info_with_reset(reset_at: DateTime<Utc>, observed_at: DateTime<Utc>) -> RateLimitInfo {
            RateLimitInfo {
                reset_at: Some(reset_at),
                observed_at,
                raw_message: "You've hit your limit".to_string(),
                source: RateLimitSource::ParsedReset,
            }
        }

        fn generic_info(observed_at: DateTime<Utc>) -> RateLimitInfo {
            RateLimitInfo {
                reset_at: None,
                observed_at,
                raw_message: "Too many requests".to_string(),
                source: RateLimitSource::Generic,
            }
        }

        #[test]
        fn waits_until_parsed_reset_plus_buffer() {
            let mut state = RateLimitState::default();
            let policy = RetryPolicy::default();
            let observed = observed_at(2026, 4, 10, 0, 15);
            let reset = observed_at(2026, 4, 10, 1, 0);

            let decision = plan_retry(&info_with_reset(reset, observed), &mut state, &policy);

            assert_eq!(
                decision,
                RetryDecision::Wait(RetryWait {
                    duration: Duration::from_secs(45 * 60 + 60),
                    retry_at: observed_at(2026, 4, 10, 1, 1),
                    reason: RetryReason::ParsedReset,
                    active_reset_at: Some(reset),
                    post_reset_failures: 0,
                })
            );
        }

        #[test]
        fn falls_back_when_no_reset_time_is_available() {
            let mut state = RateLimitState::default();
            let policy = RetryPolicy::default();
            let observed = observed_at(2026, 4, 10, 0, 15);

            let decision = plan_retry(&generic_info(observed), &mut state, &policy);

            assert_eq!(
                decision,
                RetryDecision::Wait(RetryWait {
                    duration: Duration::from_secs(DEFAULT_FALLBACK_SECONDS),
                    retry_at: observed_at(2026, 4, 10, 0, 16),
                    reason: RetryReason::Fallback,
                    active_reset_at: None,
                    post_reset_failures: 0,
                })
            );
        }

        #[test]
        fn keeps_waiting_toward_known_future_reset_when_new_message_has_no_reset() {
            let mut state = RateLimitState {
                active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                post_reset_failures: 0,
            };
            let policy = RetryPolicy::default();
            let observed = observed_at(2026, 4, 10, 0, 30);

            let decision = plan_retry(&generic_info(observed), &mut state, &policy);

            assert_eq!(
                decision,
                RetryDecision::Wait(RetryWait {
                    duration: Duration::from_secs(31 * 60),
                    retry_at: observed_at(2026, 4, 10, 1, 1),
                    reason: RetryReason::ExistingReset,
                    active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                    post_reset_failures: 0,
                })
            );
        }

        #[test]
        fn applies_post_reset_backoff_after_expected_reset_was_honored() {
            let mut state = RateLimitState {
                active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                post_reset_failures: 0,
            };
            let policy = RetryPolicy::default();
            let observed = observed_at(2026, 4, 10, 1, 2);

            let first = plan_retry(&generic_info(observed), &mut state, &policy);
            assert_eq!(
                first,
                RetryDecision::Wait(RetryWait {
                    duration: Duration::from_secs(5 * 60),
                    retry_at: observed_at(2026, 4, 10, 1, 7),
                    reason: RetryReason::PostResetBackoff,
                    active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                    post_reset_failures: 1,
                })
            );

            let second = plan_retry(
                &generic_info(observed_at(2026, 4, 10, 1, 8)),
                &mut state,
                &policy,
            );
            assert_eq!(
                second,
                RetryDecision::Wait(RetryWait {
                    duration: Duration::from_secs(15 * 60),
                    retry_at: observed_at(2026, 4, 10, 1, 23),
                    reason: RetryReason::PostResetBackoff,
                    active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                    post_reset_failures: 2,
                })
            );

            let third = plan_retry(
                &generic_info(observed_at(2026, 4, 10, 1, 24)),
                &mut state,
                &policy,
            );
            assert_eq!(
                third,
                RetryDecision::Wait(RetryWait {
                    duration: Duration::from_secs(30 * 60),
                    retry_at: observed_at(2026, 4, 10, 1, 54),
                    reason: RetryReason::PostResetBackoff,
                    active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                    post_reset_failures: 3,
                })
            );
        }

        #[test]
        fn aborts_after_too_many_post_reset_failures() {
            let mut state = RateLimitState {
                active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                post_reset_failures: DEFAULT_POST_RESET_MAX_RETRIES - 1,
            };
            let policy = RetryPolicy::default();
            let observed = observed_at(2026, 4, 10, 2, 0);

            let decision = plan_retry(&generic_info(observed), &mut state, &policy);

            assert_eq!(
                decision,
                RetryDecision::Abort(RetryAbort {
                    active_reset_at: Some(observed_at(2026, 4, 10, 1, 0)),
                    post_reset_failures: DEFAULT_POST_RESET_MAX_RETRIES,
                    last_message: "Too many requests".to_string(),
                })
            );
        }

        #[test]
        fn post_reset_backoff_is_capped() {
            assert_eq!(post_reset_backoff(3, 10 * 60), Duration::from_secs(10 * 60));
        }
    }
}
