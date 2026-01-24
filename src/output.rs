use owo_colors::OwoColorize;
use std::time::Duration;

const PREFIX: &str = "[ralph]";

#[must_use]
pub fn format_duration(d: Duration) -> String {
    format!("{}m {}s", d.as_secs() / 60, d.as_secs() % 60)
}

pub fn log(msg: &str) {
    println!("{} {}", PREFIX.blue(), msg);
}

pub fn success(msg: &str) {
    println!("{} {}", PREFIX.green(), msg);
}

pub fn warn(msg: &str) {
    println!("{} {}", PREFIX.yellow(), msg);
}

pub fn error(msg: &str) {
    println!("{} {}", PREFIX.red(), msg);
}

pub fn dim(msg: &str) {
    println!("{} {}", PREFIX.cyan(), msg.dimmed());
}

pub fn header(msg: &str) {
    println!("{} {}", PREFIX.blue().bold(), msg.bold());
}

pub fn separator() {
    header("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}

pub fn section(title: &str) {
    separator();
    header(title);
    separator();
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_zero() {
        let d = Duration::from_secs(0);
        assert_eq!(format_duration(d), "0m 0s");
    }

    #[test]
    fn format_duration_seconds_only() {
        let d = Duration::from_secs(45);
        assert_eq!(format_duration(d), "0m 45s");
    }

    #[test]
    fn format_duration_one_minute() {
        let d = Duration::from_secs(60);
        assert_eq!(format_duration(d), "1m 0s");
    }

    #[test]
    fn format_duration_minutes_and_seconds() {
        let d = Duration::from_secs(90);
        assert_eq!(format_duration(d), "1m 30s");
    }

    #[test]
    fn format_duration_one_hour_one_second() {
        let d = Duration::from_secs(3661);
        assert_eq!(format_duration(d), "61m 1s");
    }

    #[test]
    fn format_duration_large_value() {
        let d = Duration::from_secs(86400); // 24 hours
        assert_eq!(format_duration(d), "1440m 0s");
    }

    #[test]
    fn format_duration_max_u64() {
        let d = Duration::from_secs(u64::MAX);
        let mins = u64::MAX / 60;
        let secs = u64::MAX % 60;
        assert_eq!(format_duration(d), format!("{mins}m {secs}s"));
    }

    #[test]
    fn format_duration_59_seconds() {
        let d = Duration::from_secs(59);
        assert_eq!(format_duration(d), "0m 59s");
    }

    #[test]
    fn format_duration_ignores_nanos() {
        let d = Duration::new(65, 999_999_999);
        assert_eq!(format_duration(d), "1m 5s");
    }
}
