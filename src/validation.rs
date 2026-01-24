use crate::git;
use anyhow::{bail, Result};

pub(crate) fn is_diff_content_line(line: &str) -> bool {
    (line.starts_with('+') || line.starts_with('-'))
        && !line.starts_with("+++")
        && !line.starts_with("---")
}

pub(crate) fn validate_diff_content(diff: &str) -> Result<()> {
    for line in diff.lines() {
        if !is_diff_content_line(line) {
            continue;
        }

        let trimmed = line[1..].trim();

        if trimmed.is_empty() || trimmed.contains("\"status\":") {
            continue;
        }

        bail!(
            "Invalid PRD modification detected.\n\
            Only 'status' field changes are allowed.\n\
            Offending line: {}\n\
            Please revert non-status changes to the PRD.",
            line
        );
    }

    Ok(())
}

pub fn validate_prd_changes(prd_path: &str) -> Result<()> {
    let diff = git::diff_file_from_head(prd_path)?;

    if diff.is_empty() {
        return Ok(());
    }

    validate_diff_content(&diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod is_diff_content_line_tests {
        use super::*;

        #[test]
        fn added_line_is_content() {
            assert!(is_diff_content_line("+some added content"));
        }

        #[test]
        fn removed_line_is_content() {
            assert!(is_diff_content_line("-some removed content"));
        }

        #[test]
        fn plus_header_not_content() {
            assert!(!is_diff_content_line("+++ b/prd.jsonc"));
        }

        #[test]
        fn minus_header_not_content() {
            assert!(!is_diff_content_line("--- a/prd.jsonc"));
        }

        #[test]
        fn context_line_not_content() {
            assert!(!is_diff_content_line(" unchanged context"));
        }

        #[test]
        fn hunk_header_not_content() {
            assert!(!is_diff_content_line("@@ -1,5 +1,6 @@"));
        }

        #[test]
        fn diff_header_not_content() {
            assert!(!is_diff_content_line("diff --git a/prd.jsonc b/prd.jsonc"));
        }

        #[test]
        fn index_line_not_content() {
            assert!(!is_diff_content_line("index abc123..def456 100644"));
        }

        #[test]
        fn empty_line_not_content() {
            assert!(!is_diff_content_line(""));
        }

        #[test]
        fn just_plus_is_content() {
            assert!(is_diff_content_line("+"));
        }

        #[test]
        fn just_minus_is_content() {
            assert!(is_diff_content_line("-"));
        }
    }

    mod validate_valid_changes {
        use super::*;

        #[test]
        fn empty_diff_is_valid() {
            assert!(validate_diff_content("").is_ok());
        }

        #[test]
        fn status_change_pending_to_in_progress() {
            let diff = r#"
diff --git a/prd.jsonc b/prd.jsonc
index abc123..def456 100644
--- a/prd.jsonc
+++ b/prd.jsonc
@@ -10,7 +10,7 @@
       "steps": ["step1"],
-      "status": "pending"
+      "status": "in-progress"
     }
"#;
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn status_change_in_progress_to_complete() {
            let diff = r#"
-      "status": "in-progress",
+      "status": "complete",
"#;
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn multiple_status_changes() {
            let diff = r#"
-      "status": "pending",
+      "status": "in-progress",
-      "status": "in-progress",
+      "status": "complete",
"#;
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn whitespace_only_changes() {
            let diff = r#"
-
+
-
+
"#;
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn status_with_surrounding_whitespace() {
            let diff = r#"
-      "status": "pending"
+      "status": "complete"
"#;
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn context_lines_ignored() {
            let diff = r#"
 context before
-      "status": "pending"
+      "status": "complete"
 context after
"#;
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn diff_headers_ignored() {
            let diff = r#"
diff --git a/prd.jsonc b/prd.jsonc
index 1234567..89abcde 100644
--- a/prd.jsonc
+++ b/prd.jsonc
@@ -1,3 +1,3 @@
-      "status": "pending"
+      "status": "complete"
"#;
            assert!(validate_diff_content(diff).is_ok());
        }
    }

    mod validate_rejected_changes {
        use super::*;

        #[test]
        fn description_change_rejected() {
            let diff = r#"
-      "description": "old description"
+      "description": "new description"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Invalid PRD modification"));
            assert!(err.contains("description"));
        }

        #[test]
        fn id_change_rejected() {
            let diff = r#"
-      "id": "old-id"
+      "id": "new-id"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn steps_change_rejected() {
            let diff = r#"
-      "steps": ["step1"]
+      "steps": ["step1", "step2"]
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn notes_change_rejected() {
            let diff = r#"
-      "notes": "old notes"
+      "notes": "new notes"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn category_change_rejected() {
            let diff = r#"
-      "category": "feature"
+      "category": "test"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn new_feature_addition_rejected() {
            let diff = r#"
+    {
+      "id": "new-feature",
+      "description": "A new feature"
+    }
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn feature_removal_rejected() {
            let diff = r#"
-    {
-      "id": "removed-feature",
-      "description": "Going away"
-    }
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn project_name_change_rejected() {
            let diff = r#"
-    "name": "old-project"
+    "name": "new-project"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn error_message_contains_offending_line() {
            let diff = r#"
+      "malicious": "content"
"#;
            let result = validate_diff_content(diff);
            let err = result.unwrap_err().to_string();
            assert!(err.contains("\"malicious\": \"content\""));
        }

        #[test]
        fn mixed_valid_and_invalid_rejected() {
            let diff = r#"
-      "status": "pending"
+      "status": "in-progress"
-      "description": "old"
+      "description": "new"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn code_content_rejected() {
            let diff = r#"
+println!("Hello, world!");
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }
    }

    mod edge_cases {
        use super::*;

        #[test]
        fn status_in_string_value_not_treated_as_field() {
            // "status": appears inside a string value, not as a JSON field
            let diff = r#"
-      "status": "pending"
+      "status": "in-progress: status update"
"#;
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn status_field_in_nested_quotes_rejected() {
            // Adding content that contains "status": as part of a description
            let diff = r#"
+      "description": "Set \"status\": \"done\" in config"
"#;
            assert!(validate_diff_content(diff).is_err());
        }

        #[test]
        fn status_keyword_in_other_context_rejected() {
            // "status" appears but not as a field name
            let diff = r#"
+      "description": "Check status bar"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn unicode_content_rejected() {
            let diff = r#"
+      "description": "🚀 New feature"
"#;
            let result = validate_diff_content(diff);
            assert!(result.is_err());
        }

        #[test]
        fn very_long_status_line() {
            let spaces = " ".repeat(1000);
            let diff = format!(
                "-{}\"status\": \"pending\"\n+{}\"status\": \"complete\"\n",
                spaces, spaces
            );
            assert!(validate_diff_content(&diff).is_ok());
        }

        #[test]
        fn newline_only_additions() {
            let diff = "+\n+\n+\n";
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn tab_indentation_status_change() {
            let diff = "-\t\"status\": \"pending\"\n+\t\"status\": \"complete\"\n";
            assert!(validate_diff_content(diff).is_ok());
        }

        #[test]
        fn raw_content_without_diff_headers() {
            // Handles diff output that jumps straight to content lines
            let diff = "-old line\n+new line\n";
            assert!(validate_diff_content(diff).is_err());
        }

        #[test]
        fn binary_diff_marker_ignored() {
            let diff = "Binary files a/image.png and b/image.png differ\n";
            assert!(validate_diff_content(diff).is_ok());
        }
    }
}
