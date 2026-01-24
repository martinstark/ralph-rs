use anyhow::{Context, Result};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct GitStatus {
    pub branch: String,
    pub uncommitted_changes: usize,
}

#[must_use]
pub fn get_git_status() -> Option<GitStatus> {
    if !is_git_repo() {
        return None;
    }
    Some(GitStatus {
        branch: current_branch().unwrap_or_else(|_| "unknown".into()),
        uncommitted_changes: uncommitted_changes_count().unwrap_or(0),
    })
}

#[must_use]
pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .context("Failed to get current branch")?;

    Ok(parse_branch_output(&String::from_utf8_lossy(&output.stdout)))
}

pub(crate) fn parse_branch_output(output: &str) -> String {
    output.trim().to_string()
}

pub fn uncommitted_changes_count() -> Result<usize> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("Failed to get git status")?;

    Ok(parse_porcelain_status(&String::from_utf8_lossy(&output.stdout)))
}

pub(crate) fn parse_porcelain_status(output: &str) -> usize {
    output.lines().filter(|l| !l.is_empty()).count()
}

pub fn recent_commits(count: usize) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["log", "--oneline", &format!("-{count}")])
        .output()
        .context("Failed to get git log")?;

    Ok(parse_log_output(&String::from_utf8_lossy(&output.stdout)))
}

pub(crate) fn parse_log_output(output: &str) -> Vec<String> {
    output.lines().map(String::from).collect()
}

pub fn diff_file_from_head(path: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["diff", "HEAD", "--", path])
        .output()
        .context("Failed to get git diff from HEAD")?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_branch_output_simple() {
        assert_eq!(parse_branch_output("main\n"), "main");
    }

    #[test]
    fn parse_branch_output_with_trailing_newline() {
        assert_eq!(parse_branch_output("feature/test\n"), "feature/test");
    }

    #[test]
    fn parse_branch_output_with_leading_whitespace() {
        assert_eq!(parse_branch_output("  develop\n"), "develop");
    }

    #[test]
    fn parse_branch_output_with_multiple_newlines() {
        assert_eq!(parse_branch_output("main\n\n"), "main");
    }

    #[test]
    fn parse_branch_output_empty() {
        assert_eq!(parse_branch_output(""), "");
    }

    #[test]
    fn parse_branch_output_only_whitespace() {
        assert_eq!(parse_branch_output("   \n"), "");
    }

    #[test]
    fn parse_branch_output_complex_branch_name() {
        assert_eq!(
            parse_branch_output("feature/JIRA-123-add-thing\n"),
            "feature/JIRA-123-add-thing"
        );
    }

    #[test]
    fn parse_branch_output_no_trailing_newline() {
        assert_eq!(parse_branch_output("main"), "main");
    }

    #[test]
    fn parse_porcelain_status_empty() {
        assert_eq!(parse_porcelain_status(""), 0);
    }

    #[test]
    fn parse_porcelain_status_clean() {
        assert_eq!(parse_porcelain_status("\n"), 0);
    }

    #[test]
    fn parse_porcelain_status_one_modified() {
        assert_eq!(parse_porcelain_status(" M src/main.rs\n"), 1);
    }

    #[test]
    fn parse_porcelain_status_one_added() {
        assert_eq!(parse_porcelain_status("A  new_file.txt\n"), 1);
    }

    #[test]
    fn parse_porcelain_status_one_deleted() {
        assert_eq!(parse_porcelain_status(" D old_file.txt\n"), 1);
    }

    #[test]
    fn parse_porcelain_status_one_untracked() {
        assert_eq!(parse_porcelain_status("?? untracked.txt\n"), 1);
    }

    #[test]
    fn parse_porcelain_status_multiple_changes() {
        let output = " M src/main.rs\n M src/lib.rs\nA  new.txt\n?? untracked.txt\n";
        assert_eq!(parse_porcelain_status(output), 4);
    }

    #[test]
    fn parse_porcelain_status_staged_and_unstaged() {
        let output = "MM src/main.rs\n";
        assert_eq!(parse_porcelain_status(output), 1);
    }

    #[test]
    fn parse_porcelain_status_renamed() {
        let output = "R  old.rs -> new.rs\n";
        assert_eq!(parse_porcelain_status(output), 1);
    }

    #[test]
    fn parse_porcelain_status_with_spaces_in_filename() {
        let output = " M \"file with spaces.txt\"\n";
        assert_eq!(parse_porcelain_status(output), 1);
    }

    #[test]
    fn parse_porcelain_status_mixed_empty_lines() {
        let output = " M file1.txt\n\n M file2.txt\n\n";
        assert_eq!(parse_porcelain_status(output), 2);
    }

    #[test]
    fn parse_log_output_empty() {
        assert_eq!(parse_log_output(""), Vec::<String>::new());
    }

    #[test]
    fn parse_log_output_single_commit() {
        let output = "abc1234 Initial commit\n";
        assert_eq!(parse_log_output(output), vec!["abc1234 Initial commit"]);
    }

    #[test]
    fn parse_log_output_multiple_commits() {
        let output = "abc1234 Third commit\ndef5678 Second commit\nghi9012 First commit\n";
        assert_eq!(
            parse_log_output(output),
            vec![
                "abc1234 Third commit",
                "def5678 Second commit",
                "ghi9012 First commit"
            ]
        );
    }

    #[test]
    fn parse_log_output_no_trailing_newline() {
        let output = "abc1234 Commit message";
        assert_eq!(parse_log_output(output), vec!["abc1234 Commit message"]);
    }

    #[test]
    fn parse_log_output_commit_with_special_chars() {
        let output = "abc1234 fix: handle edge case (JIRA-123)\n";
        assert_eq!(
            parse_log_output(output),
            vec!["abc1234 fix: handle edge case (JIRA-123)"]
        );
    }
}
