use crate::prd::Prd;
use std::path::Path;

#[must_use]
pub fn build_system_prompt(prd: &Prd, prd_path: &Path, progress_path: &Path) -> String {
    let prd_content =
        std::fs::read_to_string(prd_path).unwrap_or_else(|_| "Failed to read PRD".to_string());

    let verification_commands: String = prd
        .verification
        .commands
        .iter()
        .map(|cmd| format!("- `{}` - {}", cmd.command, cmd.description))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"You are working autonomously in a Ralph loop. Each iteration is a fresh context window.

## Important Paths

- **PRD file**: {prd_path}
- **Progress file**: {progress_path}

## Rules

1. **ONE feature per session** - Focus on a single feature from the PRD
2. **Status-only edits** - You may ONLY change the "status" field in {prd_path}
3. **No test removal** - Never remove or weaken existing tests
4. **Verify before complete** - Run all verification commands before marking complete
5. **Commit per feature** - Commit changes with descriptive messages, include only files relevant to the feature

## Verification Commands

Run these commands to verify your changes:
{verification_commands}

## Workflow

1. Read {prd_path} and {progress_path} for context
2. Find the first feature with status "pending" or "in-progress"
3. If "pending", update status to "in-progress"
4. Implement the feature following the defined steps
5. Run verification commands
6. If verification passes, update feature status to "complete"
7. Commit your changes with a descriptive message (only feature-related files)
8. **ALWAYS** append to {progress_path} at the end of each loop, documenting:
   - Which feature you worked on
   - What you accomplished
   - Any blockers or issues encountered
   - Current status
9. **STOP** - Do not start another feature. The next iteration will handle remaining work.

## Completion

When ALL features have status "complete" and all verifications pass:
1. Append final summary to {progress_path}
2. Make a final commit
3. Output: {completion_marker}

## Current PRD

{prd_content}
"#,
        prd_path = prd_path.display(),
        progress_path = progress_path.display(),
        verification_commands = verification_commands,
        completion_marker = prd.completion.marker,
        prd_content = prd_content,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prd::{Category, Completion, Feature, Project, Status, Verification, VerifyCommand};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_test_prd(commands: Vec<VerifyCommand>, marker: &str) -> Prd {
        Prd {
            project: Project {
                name: "test-project".into(),
                description: "A test project".into(),
                repository: None,
            },
            verification: Verification {
                commands,
                run_after_each_feature: true,
            },
            features: vec![Feature {
                id: "feat-1".into(),
                category: Category::Functional,
                description: "Test feature".into(),
                steps: vec!["Step 1".into()],
                status: Status::Pending,
                notes: None,
            }],
            completion: Completion {
                all_features_complete: true,
                all_verifications_passing: true,
                marker: marker.into(),
            },
        }
    }

    mod output_structure_tests {
        use super::*;

        #[test]
        fn contains_important_paths_section() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Important Paths"));
            assert!(result.contains("**PRD file**"));
            assert!(result.contains("**Progress file**"));
        }

        #[test]
        fn contains_rules_section() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Rules"));
            assert!(result.contains("ONE feature per session"));
            assert!(result.contains("Status-only edits"));
            assert!(result.contains("No test removal"));
            assert!(result.contains("Verify before complete"));
            assert!(result.contains("Commit per feature"));
        }

        #[test]
        fn contains_workflow_section() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Workflow"));
            assert!(result.contains("Find the first feature"));
            assert!(result.contains("Run verification commands"));
            assert!(result.contains("Commit your changes"));
            assert!(result.contains("**STOP**"));
        }

        #[test]
        fn contains_completion_section() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Completion"));
            assert!(result.contains("When ALL features have status"));
        }

        #[test]
        fn contains_current_prd_section() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Current PRD"));
        }

        #[test]
        fn includes_prd_path_in_output() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();
            let prd_path = prd_file.path();

            let result = build_system_prompt(&prd, prd_path, Path::new("progress.txt"));

            assert!(result.contains(&prd_path.display().to_string()));
        }

        #[test]
        fn includes_progress_path_in_output() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("./my-progress.txt"));

            assert!(result.contains("./my-progress.txt"));
        }
    }

    mod prd_content_tests {
        use super::*;

        #[test]
        fn embeds_prd_file_content() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            let prd_content = r#"{"project": {"name": "embedded-test"}}"#;
            write!(prd_file, "{}", prd_content).unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains(prd_content));
        }

        #[test]
        fn handles_multiline_prd_content() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            let prd_content = "line1\nline2\nline3";
            write!(prd_file, "{}", prd_content).unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("line1\nline2\nline3"));
        }

        #[test]
        fn handles_unicode_prd_content() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            let prd_content = "项目: テスト 🚀";
            write!(prd_file, "{}", prd_content).unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains(prd_content));
        }

        #[test]
        fn handles_missing_prd_file() {
            let prd = make_test_prd(vec![], "DONE");

            let result = build_system_prompt(&prd, Path::new("/nonexistent/prd.json"), Path::new("progress.txt"));

            assert!(result.contains("Failed to read PRD"));
        }

        #[test]
        fn includes_completion_marker_from_prd() {
            let prd = make_test_prd(vec![], "<promise>COMPLETE</promise>");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("<promise>COMPLETE</promise>"));
        }

        #[test]
        fn custom_completion_marker_is_used() {
            let prd = make_test_prd(vec![], "CUSTOM_MARKER_12345");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("CUSTOM_MARKER_12345"));
        }
    }

    mod verification_commands_tests {
        use super::*;

        #[test]
        fn includes_verification_commands_section() {
            let prd = make_test_prd(
                vec![VerifyCommand {
                    name: "test".into(),
                    command: "cargo test".into(),
                    description: "Run tests".into(),
                }],
                "DONE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Verification Commands"));
        }

        #[test]
        fn formats_single_command_correctly() {
            let prd = make_test_prd(
                vec![VerifyCommand {
                    name: "check".into(),
                    command: "cargo check".into(),
                    description: "Type checking".into(),
                }],
                "DONE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("- `cargo check` - Type checking"));
        }

        #[test]
        fn formats_multiple_commands_correctly() {
            let prd = make_test_prd(
                vec![
                    VerifyCommand {
                        name: "check".into(),
                        command: "cargo check".into(),
                        description: "Type checking".into(),
                    },
                    VerifyCommand {
                        name: "test".into(),
                        command: "cargo test".into(),
                        description: "Run tests".into(),
                    },
                    VerifyCommand {
                        name: "lint".into(),
                        command: "cargo clippy".into(),
                        description: "Lint code".into(),
                    },
                ],
                "DONE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("- `cargo check` - Type checking"));
            assert!(result.contains("- `cargo test` - Run tests"));
            assert!(result.contains("- `cargo clippy` - Lint code"));
        }

        #[test]
        fn handles_empty_commands_list() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Verification Commands"));
            assert!(result.contains("Run these commands to verify"));
        }

        #[test]
        fn handles_commands_with_special_characters() {
            let prd = make_test_prd(
                vec![VerifyCommand {
                    name: "clippy".into(),
                    command: "cargo clippy -- -D warnings".into(),
                    description: "Lint with warnings as errors".into(),
                }],
                "DONE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("- `cargo clippy -- -D warnings` - Lint with warnings as errors"));
        }

        #[test]
        fn handles_commands_with_pipes() {
            let prd = make_test_prd(
                vec![VerifyCommand {
                    name: "count".into(),
                    command: "wc -l src/*.rs | tail -1".into(),
                    description: "Count lines".into(),
                }],
                "DONE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("- `wc -l src/*.rs | tail -1` - Count lines"));
        }
    }

    mod edge_case_tests {
        use super::*;

        #[test]
        fn handles_empty_prd_file() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("## Current PRD"));
        }

        #[test]
        fn handles_very_long_prd_content() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            let long_content = "x".repeat(100_000);
            write!(prd_file, "{}", long_content).unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains(&long_content));
        }

        #[test]
        fn handles_paths_with_spaces() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("path with spaces/progress.txt"));

            assert!(result.contains("path with spaces/progress.txt"));
        }

        #[test]
        fn handles_absolute_paths() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = build_system_prompt(&prd, prd_file.path(), Path::new("/absolute/path/progress.txt"));

            assert!(result.contains("/absolute/path/progress.txt"));
        }
    }
}
