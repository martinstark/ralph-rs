use crate::{output, prd};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub struct FeatureRetryTracker {
    counts: HashMap<String, u32>,
    max_retries: u32,
}

impl FeatureRetryTracker {
    #[must_use]
    pub fn new(max_retries: u32) -> Self {
        Self {
            counts: HashMap::new(),
            max_retries,
        }
    }

    pub fn record_failure(&mut self, feature_id: &str) -> u32 {
        let count = self.counts.entry(feature_id.to_string()).or_insert(0);
        *count += 1;
        *count
    }

    pub fn reset(&mut self, feature_id: &str) {
        self.counts.remove(feature_id);
    }

    #[must_use]
    pub fn should_block(&self, feature_id: &str) -> bool {
        if self.max_retries == 0 {
            return false;
        }
        self.counts
            .get(feature_id)
            .is_some_and(|&c| c >= self.max_retries)
    }

    #[must_use]
    pub fn get_count(&self, feature_id: &str) -> u32 {
        self.counts.get(feature_id).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.max_retries > 0
    }
}

pub fn get_current_feature_id(prd: &prd::Prd) -> Option<String> {
    prd.features
        .iter()
        .find(|f| f.status == prd::Status::InProgress)
        .map(|f| f.id.clone())
}

pub fn update_feature_status_to_blocked(prd_path: &Path, feature_id: &str) -> Result<()> {
    let content = std::fs::read_to_string(prd_path)
        .with_context(|| format!("Failed to read PRD file: {}", prd_path.display()))?;

    let pattern = format!(r#""id": "{}""#, feature_id);
    if !content.contains(&pattern) {
        anyhow::bail!("Feature {} not found in PRD", feature_id);
    }

    let updated = update_status_in_content(&content, feature_id);

    std::fs::write(prd_path, updated)
        .with_context(|| format!("Failed to write PRD file: {}", prd_path.display()))?;

    output::warn(&format!(
        "Feature '{}' auto-blocked after max retries",
        feature_id
    ));

    Ok(())
}

fn update_status_in_content(content: &str, feature_id: &str) -> String {
    let mut result = String::new();
    let mut in_target_feature = false;
    let mut status_updated = false;
    let id_pattern = format!(r#""id": "{}""#, feature_id);

    for line in content.lines() {
        if line.contains(&id_pattern) {
            in_target_feature = true;
        }

        if in_target_feature && !status_updated && line.contains(r#""status":"#) {
            let updated_line = line
                .replace(r#""status": "in-progress""#, r#""status": "blocked""#)
                .replace(r#""status": "pending""#, r#""status": "blocked""#)
                .replace(r#""status":"in-progress""#, r#""status": "blocked""#)
                .replace(r#""status":"pending""#, r#""status": "blocked""#);
            result.push_str(&updated_line);
            status_updated = true;
            in_target_feature = false;
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }

    if result.ends_with('\n') && !content.ends_with('\n') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    mod feature_retry_tracker_tests {
        use super::*;

        #[test]
        fn new_creates_empty_tracker() {
            let tracker = FeatureRetryTracker::new(3);
            assert_eq!(tracker.get_count("any-feature"), 0);
        }

        #[test]
        fn record_failure_increments_count() {
            let mut tracker = FeatureRetryTracker::new(3);
            assert_eq!(tracker.record_failure("feat-1"), 1);
            assert_eq!(tracker.record_failure("feat-1"), 2);
            assert_eq!(tracker.record_failure("feat-1"), 3);
            assert_eq!(tracker.get_count("feat-1"), 3);
        }

        #[test]
        fn tracks_multiple_features_independently() {
            let mut tracker = FeatureRetryTracker::new(3);
            tracker.record_failure("feat-1");
            tracker.record_failure("feat-1");
            tracker.record_failure("feat-2");

            assert_eq!(tracker.get_count("feat-1"), 2);
            assert_eq!(tracker.get_count("feat-2"), 1);
        }

        #[test]
        fn reset_clears_feature_count() {
            let mut tracker = FeatureRetryTracker::new(3);
            tracker.record_failure("feat-1");
            tracker.record_failure("feat-1");
            tracker.reset("feat-1");

            assert_eq!(tracker.get_count("feat-1"), 0);
        }

        #[test]
        fn should_block_returns_true_at_max() {
            let mut tracker = FeatureRetryTracker::new(3);
            tracker.record_failure("feat-1");
            tracker.record_failure("feat-1");
            assert!(!tracker.should_block("feat-1"));

            tracker.record_failure("feat-1");
            assert!(tracker.should_block("feat-1"));
        }

        #[test]
        fn should_block_returns_false_when_disabled() {
            let mut tracker = FeatureRetryTracker::new(0);
            tracker.record_failure("feat-1");
            tracker.record_failure("feat-1");
            tracker.record_failure("feat-1");
            tracker.record_failure("feat-1");

            assert!(!tracker.should_block("feat-1"));
        }

        #[test]
        fn is_enabled_returns_true_when_max_positive() {
            let tracker = FeatureRetryTracker::new(3);
            assert!(tracker.is_enabled());
        }

        #[test]
        fn is_enabled_returns_false_when_max_zero() {
            let tracker = FeatureRetryTracker::new(0);
            assert!(!tracker.is_enabled());
        }
    }

    mod update_status_tests {
        use super::*;

        #[test]
        fn updates_in_progress_to_blocked() {
            let content = r#"{
  "features": [
    {
      "id": "feat-1",
      "status": "in-progress"
    }
  ]
}"#;
            let result = update_status_in_content(content, "feat-1");
            assert!(result.contains(r#""status": "blocked""#));
            assert!(!result.contains(r#""in-progress""#));
        }

        #[test]
        fn updates_pending_to_blocked() {
            let content = r#"{
  "features": [
    {
      "id": "feat-1",
      "status": "pending"
    }
  ]
}"#;
            let result = update_status_in_content(content, "feat-1");
            assert!(result.contains(r#""status": "blocked""#));
            assert!(!result.contains(r#""pending""#));
        }

        #[test]
        fn only_updates_target_feature() {
            let content = r#"{
  "features": [
    {
      "id": "feat-1",
      "status": "in-progress"
    },
    {
      "id": "feat-2",
      "status": "pending"
    }
  ]
}"#;
            let result = update_status_in_content(content, "feat-1");
            assert!(result.contains(r#""status": "blocked""#));
            assert!(result.contains(r#""status": "pending""#));
        }

        #[test]
        fn handles_no_space_format() {
            let content = r#"{"id": "feat-1","status":"in-progress"}"#;
            let result = update_status_in_content(content, "feat-1");
            assert!(result.contains(r#""status": "blocked""#));
        }

        #[test]
        fn leaves_other_features_unchanged() {
            let content = r#"{
  "features": [
    { "id": "feat-1", "status": "complete" },
    { "id": "feat-2", "status": "in-progress" }
  ]
}"#;
            let result = update_status_in_content(content, "feat-2");
            assert!(result.contains(r#""status": "complete""#));
            assert!(result.contains(r#""status": "blocked""#));
        }
    }

    mod get_current_feature_tests {
        use super::*;
        use std::io::Write;
        use tempfile::NamedTempFile;

        fn create_test_prd(content: &str) -> prd::Prd {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", content).unwrap();
            prd::Prd::load(file.path()).unwrap()
        }

        #[test]
        fn returns_in_progress_feature() {
            let prd = create_test_prd(
                r#"{
                "project": { "name": "test", "description": "d" },
                "verification": { "commands": [], "runAfterEachFeature": true },
                "features": [
                    { "id": "feat-1", "category": "functional", "description": "d", "steps": [], "status": "complete" },
                    { "id": "feat-2", "category": "functional", "description": "d", "steps": [], "status": "in-progress" }
                ],
                "completion": { "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "X" }
            }"#,
            );

            assert_eq!(get_current_feature_id(&prd), Some("feat-2".to_string()));
        }

        #[test]
        fn returns_none_when_no_in_progress() {
            let prd = create_test_prd(
                r#"{
                "project": { "name": "test", "description": "d" },
                "verification": { "commands": [], "runAfterEachFeature": true },
                "features": [
                    { "id": "feat-1", "category": "functional", "description": "d", "steps": [], "status": "complete" },
                    { "id": "feat-2", "category": "functional", "description": "d", "steps": [], "status": "pending" }
                ],
                "completion": { "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "X" }
            }"#,
            );

            assert_eq!(get_current_feature_id(&prd), None);
        }
    }
}
