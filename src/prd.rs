use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
pub struct Prd {
    pub project: Project,
    pub verification: Verification,
    pub features: Vec<Feature>,
    pub completion: Completion,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Project {
    pub name: String,
    pub description: String,
    pub repository: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Verification {
    pub commands: Vec<VerifyCommand>,
    #[serde(rename = "runAfterEachFeature")]
    pub run_after_each_feature: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VerifyCommand {
    pub name: String,
    pub command: String,
    pub description: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Feature {
    pub id: String,
    pub category: String,
    pub description: String,
    pub steps: Vec<String>,
    pub status: Status,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone, Copy, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pending,
    InProgress,
    Complete,
    Blocked,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Completion {
    #[serde(rename = "allFeaturesComplete")]
    pub all_features_complete: bool,
    #[serde(rename = "allVerificationsPassing")]
    pub all_verifications_passing: bool,
    pub marker: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StatusCounts {
    pub pending: usize,
    pub in_progress: usize,
    pub complete: usize,
    pub blocked: usize,
}

impl Prd {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read PRD file: {}", path.display()))?;

        let prd: Prd = json5::from_str(&content)
            .with_context(|| format!("Failed to parse PRD file: {}", path.display()))?;

        Ok(prd)
    }

    #[must_use]
    pub fn status_counts(&self) -> StatusCounts {
        self.features.iter().fold(StatusCounts::default(), |mut c, f| {
            match f.status {
                Status::Pending => c.pending += 1,
                Status::InProgress => c.in_progress += 1,
                Status::Complete => c.complete += 1,
                Status::Blocked => c.blocked += 1,
            }
            c
        })
    }
}

pub fn generate_template(path: &Path) -> Result<()> {
    generate_template_content(path, DEFAULT_TEMPLATE)
}

fn generate_template_content(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write PRD template to: {}", path.display()))?;
    Ok(())
}

const DEFAULT_TEMPLATE: &str = r#"{
  // PRD (Product Requirements Document) for Ralph autonomous loop
  // Edit this file to define your features and verification commands.
  //
  // RULES FOR THE AGENT:
  // 1. Work on ONE feature per session
  // 2. You may ONLY update the "status" field of features
  // 3. Run verification tests before marking any feature complete
  // 4. Commit changes with descriptive messages

  "project": {
    "name": "my-project",
    "description": "Description of your project"
  },

  "verification": {
    "commands": [
      {
        "name": "check",
        "command": "echo 'Add your check command here'",
        "description": "Type checking / compilation"
      },
      {
        "name": "lint",
        "command": "echo 'Add your lint command here'",
        "description": "Linting and formatting"
      },
      {
        "name": "test",
        "command": "echo 'Add your test command here'",
        "description": "Run test suite"
      }
    ],
    "runAfterEachFeature": true
  },

  "features": [
    {
      "id": "example-feature",
      "category": "functional",
      "description": "Brief description of what needs to be done",
      "steps": [
        "Step 1: First action",
        "Step 2: Second action",
        "Step 3: Run verification"
      ],
      "status": "pending",
      "notes": "Optional notes or context"
    }
  ],

  "completion": {
    "allFeaturesComplete": true,
    "allVerificationsPassing": true,
    "marker": "<promise>COMPLETE</promise>"
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn minimal_prd_json5() -> &'static str {
        r#"{
            "project": { "name": "test", "description": "desc" },
            "verification": { "commands": [], "runAfterEachFeature": true },
            "features": [],
            "completion": { "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "DONE" }
        }"#
    }

    fn full_prd_json5() -> &'static str {
        r#"{
            // comment
            "project": { "name": "my-project", "description": "A project", "repository": "https://github.com/example/repo" },
            "verification": {
                "commands": [
                    { "name": "check", "command": "cargo check", "description": "Type check" },
                ],
                "runAfterEachFeature": false,
            },
            "features": [
                { "id": "feat-1", "category": "functional", "description": "First", "steps": ["step1"], "status": "pending", "notes": "note" },
                { "id": "feat-2", "category": "bugfix", "description": "Second", "steps": [], "status": "in-progress" },
                { "id": "feat-3", "category": "refactor", "description": "Third", "steps": [], "status": "complete" },
                { "id": "feat-4", "category": "test", "description": "Fourth", "steps": [], "status": "blocked" },
                { "id": "feat-5", "category": "docs", "description": "Fifth", "steps": [], "status": "pending" },
            ],
            "completion": { "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "<promise>COMPLETE</promise>" },
        }"#
    }

    mod load_tests {
        use super::*;

        #[test]
        fn loads_minimal_prd() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", minimal_prd_json5()).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            assert_eq!(prd.project.name, "test");
            assert_eq!(prd.project.description, "desc");
            assert!(prd.features.is_empty());
        }

        #[test]
        fn loads_full_prd_with_comments_and_trailing_commas() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", full_prd_json5()).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            assert_eq!(prd.project.name, "my-project");
            assert_eq!(prd.project.repository, Some("https://github.com/example/repo".into()));
            assert_eq!(prd.verification.commands.len(), 1);
            assert!(!prd.verification.run_after_each_feature);
            assert_eq!(prd.features.len(), 5);
            assert_eq!(prd.completion.marker, "<promise>COMPLETE</promise>");
        }

        #[test]
        fn parses_all_feature_fields() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", full_prd_json5()).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            let feat = &prd.features[0];
            assert_eq!(feat.id, "feat-1");
            assert_eq!(feat.category, "functional");
            assert_eq!(feat.description, "First");
            assert_eq!(feat.steps, vec!["step1"]);
            assert_eq!(feat.status, Status::Pending);
            assert_eq!(feat.notes, Some("note".into()));
        }

        #[test]
        fn parses_optional_notes_as_none() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", full_prd_json5()).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            assert!(prd.features[1].notes.is_none());
        }

        #[test]
        fn fails_on_missing_file() {
            let result = Prd::load(Path::new("/nonexistent/path/prd.json"));
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Failed to read PRD file"));
        }

        #[test]
        fn fails_on_malformed_json() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{{ invalid json").unwrap();

            let result = Prd::load(file.path());
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Failed to parse PRD file"));
        }

        #[test]
        fn fails_on_missing_required_field() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, r#"{{ "project": {{ "name": "test" }} }}"#).unwrap();

            let result = Prd::load(file.path());
            assert!(result.is_err());
        }
    }

    mod status_counts_tests {
        use super::*;

        #[test]
        fn empty_features_returns_zeros() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", minimal_prd_json5()).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            let counts = prd.status_counts();
            assert_eq!(counts.pending, 0);
            assert_eq!(counts.in_progress, 0);
            assert_eq!(counts.complete, 0);
            assert_eq!(counts.blocked, 0);
        }

        #[test]
        fn counts_all_status_types() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", full_prd_json5()).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            let counts = prd.status_counts();
            assert_eq!(counts.pending, 2);
            assert_eq!(counts.in_progress, 1);
            assert_eq!(counts.complete, 1);
            assert_eq!(counts.blocked, 1);
        }

        #[test]
        fn counts_all_same_status() {
            let json = r#"{
                "project": { "name": "test", "description": "desc" },
                "verification": { "commands": [], "runAfterEachFeature": true },
                "features": [
                    { "id": "f1", "category": "functional", "description": "d", "steps": [], "status": "complete" },
                    { "id": "f2", "category": "functional", "description": "d", "steps": [], "status": "complete" },
                    { "id": "f3", "category": "functional", "description": "d", "steps": [], "status": "complete" }
                ],
                "completion": { "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "X" }
            }"#;
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", json).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            let counts = prd.status_counts();
            assert_eq!(counts.complete, 3);
            assert_eq!(counts.pending, 0);
            assert_eq!(counts.in_progress, 0);
            assert_eq!(counts.blocked, 0);
        }
    }

    mod serde_roundtrip_tests {
        use super::*;

        #[test]
        fn status_serializes_to_kebab_case() {
            assert_eq!(serde_json::to_string(&Status::Pending).unwrap(), "\"pending\"");
            assert_eq!(serde_json::to_string(&Status::InProgress).unwrap(), "\"in-progress\"");
            assert_eq!(serde_json::to_string(&Status::Complete).unwrap(), "\"complete\"");
            assert_eq!(serde_json::to_string(&Status::Blocked).unwrap(), "\"blocked\"");
        }

        #[test]
        fn status_deserializes_from_kebab_case() {
            assert_eq!(serde_json::from_str::<Status>("\"pending\"").unwrap(), Status::Pending);
            assert_eq!(serde_json::from_str::<Status>("\"in-progress\"").unwrap(), Status::InProgress);
            assert_eq!(serde_json::from_str::<Status>("\"complete\"").unwrap(), Status::Complete);
            assert_eq!(serde_json::from_str::<Status>("\"blocked\"").unwrap(), Status::Blocked);
        }

        #[test]
        fn status_roundtrip() {
            for status in [Status::Pending, Status::InProgress, Status::Complete, Status::Blocked] {
                let json = serde_json::to_string(&status).unwrap();
                let back: Status = serde_json::from_str(&json).unwrap();
                assert_eq!(back, status);
            }
        }

        #[test]
        fn invalid_status_fails() {
            assert!(serde_json::from_str::<Status>("\"unknown\"").is_err());
            assert!(serde_json::from_str::<Status>("\"IN_PROGRESS\"").is_err());
        }

        #[test]
        fn category_accepts_any_string() {
            let json = r#"{
                "project": { "name": "test", "description": "desc" },
                "verification": { "commands": [], "runAfterEachFeature": true },
                "features": [
                    { "id": "f1", "category": "custom-category", "description": "d", "steps": [], "status": "pending" },
                    { "id": "f2", "category": "My Feature Type", "description": "d", "steps": [], "status": "pending" }
                ],
                "completion": { "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "X" }
            }"#;
            let mut file = tempfile::NamedTempFile::new().unwrap();
            std::io::Write::write_all(&mut file, json.as_bytes()).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            assert_eq!(prd.features[0].category, "custom-category");
            assert_eq!(prd.features[1].category, "My Feature Type");
        }
    }

    mod edge_case_tests {
        use super::*;

        #[test]
        fn handles_empty_string_file() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "").unwrap();

            let result = Prd::load(file.path());
            assert!(result.is_err());
        }

        #[test]
        fn handles_whitespace_only_file() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "   \n\t  \n").unwrap();

            let result = Prd::load(file.path());
            assert!(result.is_err());
        }

        #[test]
        fn handles_comment_only_file() {
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "// just a comment").unwrap();

            let result = Prd::load(file.path());
            assert!(result.is_err());
        }

        #[test]
        fn handles_very_long_strings() {
            let long_name = "x".repeat(10000);
            let json = format!(
                r#"{{
                    "project": {{ "name": "{long_name}", "description": "desc" }},
                    "verification": {{ "commands": [], "runAfterEachFeature": true }},
                    "features": [],
                    "completion": {{ "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "X" }}
                }}"#
            );
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", json).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            assert_eq!(prd.project.name.len(), 10000);
        }

        #[test]
        fn handles_unicode_content() {
            let json = r#"{
                "project": { "name": "项目名称", "description": "🚀 Rocket description" },
                "verification": { "commands": [], "runAfterEachFeature": true },
                "features": [
                    { "id": "功能", "category": "functional", "description": "日本語テスト", "steps": ["الخطوة"], "status": "pending" }
                ],
                "completion": { "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "完成" }
            }"#;
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", json).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            assert_eq!(prd.project.name, "项目名称");
            assert_eq!(prd.features[0].id, "功能");
            assert_eq!(prd.completion.marker, "完成");
        }

        #[test]
        fn handles_many_features() {
            let features: Vec<String> = (0..100)
                .map(|i| format!(
                    r#"{{ "id": "f{i}", "category": "functional", "description": "desc", "steps": [], "status": "pending" }}"#
                ))
                .collect();
            let json = format!(
                r#"{{
                    "project": {{ "name": "test", "description": "desc" }},
                    "verification": {{ "commands": [], "runAfterEachFeature": true }},
                    "features": [{}],
                    "completion": {{ "allFeaturesComplete": true, "allVerificationsPassing": true, "marker": "X" }}
                }}"#,
                features.join(",")
            );
            let mut file = NamedTempFile::new().unwrap();
            write!(file, "{}", json).unwrap();

            let prd = Prd::load(file.path()).unwrap();
            assert_eq!(prd.features.len(), 100);
            assert_eq!(prd.status_counts().pending, 100);
        }
    }
}
