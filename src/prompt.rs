use crate::prd::Prd;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

pub const PLACEHOLDER_PRD_PATH: &str = "{prd_path}";
pub const PLACEHOLDER_PROGRESS_PATH: &str = "{progress_path}";
pub const PLACEHOLDER_VERIFICATION_COMMANDS: &str = "{verification_commands}";
pub const PLACEHOLDER_COMPLETION_MARKER: &str = "{completion_marker}";

const PROMPT_TEMPLATE: &str = r#"You are an autonomous coding agent working through features defined in a PRD.

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
7. If blocked (unclear requirements, missing dependencies, repeated failures), update status to "blocked"
8. Commit your changes with a descriptive message (only feature-related files)
9. **ALWAYS** append to {progress_path} at the end of each loop, documenting:
   - Which feature you worked on
   - What you accomplished
   - Any blockers or issues encountered
   - Current status
10. **STOP** - Do not start another feature. The next iteration will handle remaining work.

## Completion

When ALL features have status "complete" and all verifications pass:
1. Append final summary to {progress_path}
2. Make a final commit
3. Output: {completion_marker}
"#;

pub fn generate_prompt_template(path: &Path) -> Result<()> {
    fs::write(path, PROMPT_TEMPLATE)
        .with_context(|| format!("Failed to write prompt template to {}", path.display()))
}

pub fn load_custom_prompt(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read custom prompt file: {}", path.display()))
}

pub fn substitute_placeholders(
    template: &str,
    prd: &Prd,
    prd_path: &Path,
    progress_path: &Path,
) -> String {
    let verification_commands = format_verification_commands(prd);

    template
        .replace(PLACEHOLDER_PRD_PATH, &prd_path.display().to_string())
        .replace(PLACEHOLDER_PROGRESS_PATH, &progress_path.display().to_string())
        .replace(PLACEHOLDER_VERIFICATION_COMMANDS, &verification_commands)
        .replace(PLACEHOLDER_COMPLETION_MARKER, &prd.completion.marker)
}

pub fn get_system_prompt(
    prompt_path: Option<&Path>,
    prd: &Prd,
    prd_path: &Path,
    progress_path: &Path,
) -> Result<String> {
    match prompt_path {
        Some(path) => {
            let template = load_custom_prompt(path)?;
            Ok(substitute_placeholders(&template, prd, prd_path, progress_path))
        }
        None => Ok(build_system_prompt(prd, prd_path, progress_path)),
    }
}

fn format_verification_commands(prd: &Prd) -> String {
    prd.verification
        .commands
        .iter()
        .map(|cmd| format!("- `{}` - {}", cmd.command, cmd.description))
        .collect::<Vec<_>>()
        .join("\n")
}

#[must_use]
pub fn build_system_prompt(prd: &Prd, prd_path: &Path, progress_path: &Path) -> String {
    substitute_placeholders(PROMPT_TEMPLATE, prd, prd_path, progress_path)
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

    mod completion_marker_tests {
        use super::*;

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

    mod load_custom_prompt_tests {
        use super::*;

        #[test]
        fn loads_valid_file() {
            let mut file = NamedTempFile::new().unwrap();
            let content = "Custom prompt content here";
            write!(file, "{}", content).unwrap();

            let result = load_custom_prompt(file.path()).unwrap();

            assert_eq!(result, content);
        }

        #[test]
        fn returns_error_for_missing_file() {
            let result = load_custom_prompt(Path::new("/nonexistent/prompt.md"));

            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Failed to read custom prompt file"));
        }

        #[test]
        fn loads_empty_file() {
            let file = NamedTempFile::new().unwrap();

            let result = load_custom_prompt(file.path()).unwrap();

            assert_eq!(result, "");
        }

        #[test]
        fn loads_multiline_content() {
            let mut file = NamedTempFile::new().unwrap();
            let content = "Line 1\nLine 2\nLine 3";
            write!(file, "{}", content).unwrap();

            let result = load_custom_prompt(file.path()).unwrap();

            assert_eq!(result, content);
        }

        #[test]
        fn loads_unicode_content() {
            let mut file = NamedTempFile::new().unwrap();
            let content = "Unicode: 日本語 🚀 émojis";
            write!(file, "{}", content).unwrap();

            let result = load_custom_prompt(file.path()).unwrap();

            assert_eq!(result, content);
        }
    }

    mod substitute_placeholders_tests {
        use super::*;

        #[test]
        fn replaces_all_placeholders() {
            let prd = make_test_prd(
                vec![VerifyCommand {
                    name: "test".into(),
                    command: "cargo test".into(),
                    description: "Run tests".into(),
                }],
                "COMPLETE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let template = "Path: {prd_path}\nProgress: {progress_path}\nCommands:\n{verification_commands}\nMarker: {completion_marker}";
            let result = substitute_placeholders(template, &prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains(&prd_file.path().display().to_string()));
            assert!(result.contains("progress.txt"));
            assert!(result.contains("- `cargo test` - Run tests"));
            assert!(result.contains("Marker: COMPLETE"));
        }

        #[test]
        fn handles_partial_placeholders() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let template = "Only path: {prd_path} and marker: {completion_marker}";
            let result = substitute_placeholders(template, &prd, prd_file.path(), Path::new("prog.txt"));

            assert!(result.contains(&prd_file.path().display().to_string()));
            assert!(result.contains("DONE"));
            assert!(!result.contains("{prd_path}"));
            assert!(!result.contains("{completion_marker}"));
        }

        #[test]
        fn handles_no_placeholders() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let template = "Static content with no placeholders";
            let result = substitute_placeholders(template, &prd, prd_file.path(), Path::new("progress.txt"));

            assert_eq!(result, "Static content with no placeholders");
        }

        #[test]
        fn handles_repeated_placeholders() {
            let prd = make_test_prd(vec![], "MARKER");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let template = "{completion_marker} and again {completion_marker}";
            let result = substitute_placeholders(template, &prd, prd_file.path(), Path::new("progress.txt"));

            assert_eq!(result, "MARKER and again MARKER");
        }

        #[test]
        fn handles_unknown_placeholders() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let template = "Known: {completion_marker}, Unknown: {unknown_placeholder}";
            let result = substitute_placeholders(template, &prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("Known: DONE"));
            assert!(result.contains("{unknown_placeholder}"));
        }

        #[test]
        fn handles_empty_verification_commands() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let template = "Commands: {verification_commands}";
            let result = substitute_placeholders(template, &prd, prd_file.path(), Path::new("progress.txt"));

            assert_eq!(result, "Commands: ");
        }

        #[test]
        fn handles_multiple_verification_commands() {
            let prd = make_test_prd(
                vec![
                    VerifyCommand {
                        name: "check".into(),
                        command: "cargo check".into(),
                        description: "Type check".into(),
                    },
                    VerifyCommand {
                        name: "test".into(),
                        command: "cargo test".into(),
                        description: "Run tests".into(),
                    },
                ],
                "DONE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let template = "{verification_commands}";
            let result = substitute_placeholders(template, &prd, prd_file.path(), Path::new("progress.txt"));

            assert!(result.contains("- `cargo check` - Type check"));
            assert!(result.contains("- `cargo test` - Run tests"));
        }

    }

    mod get_system_prompt_tests {
        use super::*;

        #[test]
        fn uses_built_in_when_no_custom_path() {
            let prd = make_test_prd(
                vec![VerifyCommand {
                    name: "test".into(),
                    command: "cargo test".into(),
                    description: "Run tests".into(),
                }],
                "COMPLETE",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = get_system_prompt(None, &prd, prd_file.path(), Path::new("progress.txt")).unwrap();

            assert!(result.contains("## Important Paths"));
            assert!(result.contains("## Rules"));
            assert!(result.contains("## Workflow"));
        }

        #[test]
        fn uses_custom_prompt_when_path_provided() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "PRD content here").unwrap();

            let mut prompt_file = NamedTempFile::new().unwrap();
            write!(prompt_file, "Custom prompt with {{prd_path}} and {{completion_marker}}").unwrap();

            let result = get_system_prompt(
                Some(prompt_file.path()),
                &prd,
                prd_file.path(),
                Path::new("progress.txt"),
            )
            .unwrap();

            assert!(result.contains("Custom prompt with"));
            assert!(result.contains(&prd_file.path().display().to_string()));
            assert!(result.contains("DONE"));
            assert!(!result.contains("## Important Paths"));
        }

        #[test]
        fn substitutes_all_placeholders_in_custom_prompt() {
            let prd = make_test_prd(
                vec![VerifyCommand {
                    name: "check".into(),
                    command: "cargo check".into(),
                    description: "Type check".into(),
                }],
                "MARKER",
            );
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let mut prompt_file = NamedTempFile::new().unwrap();
            write!(
                prompt_file,
                "PRD: {{prd_path}}\nProgress: {{progress_path}}\nCommands:\n{{verification_commands}}\nMarker: {{completion_marker}}"
            )
            .unwrap();

            let result = get_system_prompt(
                Some(prompt_file.path()),
                &prd,
                prd_file.path(),
                Path::new("prog.txt"),
            )
            .unwrap();

            assert!(result.contains(&prd_file.path().display().to_string()));
            assert!(result.contains("prog.txt"));
            assert!(result.contains("- `cargo check` - Type check"));
            assert!(result.contains("MARKER"));
        }

        #[test]
        fn returns_error_for_missing_custom_prompt_file() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let result = get_system_prompt(
                Some(Path::new("/nonexistent/prompt.md")),
                &prd,
                prd_file.path(),
                Path::new("progress.txt"),
            );

            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Failed to read custom prompt file"));
        }

        #[test]
        fn handles_empty_custom_prompt_file() {
            let prd = make_test_prd(vec![], "DONE");
            let mut prd_file = NamedTempFile::new().unwrap();
            write!(prd_file, "{{}}").unwrap();

            let prompt_file = NamedTempFile::new().unwrap();

            let result = get_system_prompt(
                Some(prompt_file.path()),
                &prd,
                prd_file.path(),
                Path::new("progress.txt"),
            )
            .unwrap();

            assert_eq!(result, "");
        }
    }

    mod generate_prompt_template_tests {
        use super::*;
        use tempfile::TempDir;

        #[test]
        fn creates_file_with_template_content() {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("prompt.md");

            generate_prompt_template(&path).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(content.contains("You are an autonomous coding agent"));
        }

        #[test]
        fn template_contains_all_placeholders() {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("prompt.md");

            generate_prompt_template(&path).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(content.contains("{prd_path}"));
            assert!(content.contains("{progress_path}"));
            assert!(content.contains("{verification_commands}"));
            assert!(content.contains("{completion_marker}"));
        }

        #[test]
        fn template_contains_all_sections() {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("prompt.md");

            generate_prompt_template(&path).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(content.contains("## Important Paths"));
            assert!(content.contains("## Rules"));
            assert!(content.contains("## Verification Commands"));
            assert!(content.contains("## Workflow"));
            assert!(content.contains("## Completion"));
        }

        #[test]
        fn returns_error_for_invalid_path() {
            let result = generate_prompt_template(Path::new("/nonexistent/dir/prompt.md"));

            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Failed to write prompt template"));
        }

        #[test]
        fn overwrites_existing_file() {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("prompt.md");
            std::fs::write(&path, "old content").unwrap();

            generate_prompt_template(&path).unwrap();

            let content = std::fs::read_to_string(&path).unwrap();
            assert!(!content.contains("old content"));
            assert!(content.contains("You are an autonomous coding agent"));
        }
    }
}
