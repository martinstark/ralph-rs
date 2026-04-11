use anyhow::{Context, Result};
use std::process::{Command, Output};
use tokio::process::Command as TokioCommand;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct GitStatus {
    pub branch: String,
    pub uncommitted_changes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRun<T> {
    Completed(T),
    Interrupted,
}

impl<T> CommandRun<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> CommandRun<U> {
        match self {
            Self::Completed(value) => CommandRun::Completed(f(value)),
            Self::Interrupted => CommandRun::Interrupted,
        }
    }
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

pub async fn get_git_status_with_shutdown(
    cancel_token: &CancellationToken,
) -> Result<CommandRun<Option<GitStatus>>> {
    match is_git_repo_with_shutdown(cancel_token).await? {
        CommandRun::Interrupted => Ok(CommandRun::Interrupted),
        CommandRun::Completed(false) => Ok(CommandRun::Completed(None)),
        CommandRun::Completed(true) => {
            let branch = match current_branch_with_shutdown(cancel_token).await? {
                CommandRun::Completed(branch) => branch,
                CommandRun::Interrupted => return Ok(CommandRun::Interrupted),
            };
            let uncommitted_changes =
                match uncommitted_changes_count_with_shutdown(cancel_token).await? {
                    CommandRun::Completed(count) => count,
                    CommandRun::Interrupted => return Ok(CommandRun::Interrupted),
                };
            Ok(CommandRun::Completed(Some(GitStatus {
                branch,
                uncommitted_changes,
            })))
        }
    }
}

#[must_use]
pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub async fn is_git_repo_with_shutdown(
    cancel_token: &CancellationToken,
) -> Result<CommandRun<bool>> {
    Ok(run_git_command(["rev-parse", "--git-dir"], cancel_token)
        .await?
        .map(|output| output.status.success()))
}

pub fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .context("Failed to get current branch")?;

    Ok(parse_branch_output(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

pub async fn current_branch_with_shutdown(
    cancel_token: &CancellationToken,
) -> Result<CommandRun<String>> {
    Ok(run_git_command(["branch", "--show-current"], cancel_token)
        .await?
        .map(|output| parse_branch_output(&String::from_utf8_lossy(&output.stdout))))
}

pub(crate) fn parse_branch_output(output: &str) -> String {
    output.trim().to_string()
}

pub fn uncommitted_changes_count() -> Result<usize> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .context("Failed to get git status")?;

    Ok(parse_porcelain_status(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

async fn uncommitted_changes_count_with_shutdown(
    cancel_token: &CancellationToken,
) -> Result<CommandRun<usize>> {
    Ok(run_git_command(["status", "--porcelain"], cancel_token)
        .await?
        .map(|output| parse_porcelain_status(&String::from_utf8_lossy(&output.stdout))))
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

pub async fn recent_commits_with_shutdown(
    count: usize,
    cancel_token: &CancellationToken,
) -> Result<CommandRun<Vec<String>>> {
    let limit = format!("-{count}");
    Ok(
        run_git_command(["log", "--oneline", limit.as_str()], cancel_token)
            .await?
            .map(|output| parse_log_output(&String::from_utf8_lossy(&output.stdout))),
    )
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

async fn run_git_command<const N: usize>(
    args: [&str; N],
    cancel_token: &CancellationToken,
) -> Result<CommandRun<Output>> {
    if cancel_token.is_cancelled() {
        return Ok(CommandRun::Interrupted);
    }

    let command_label = format!("git {}", args.join(" "));
    let mut command = TokioCommand::new("git");
    command.args(args);
    command.kill_on_drop(true);

    tokio::select! {
        output = command.output() => {
            let output = output.with_context(|| format!("Failed to run `{command_label}`"))?;
            Ok(CommandRun::Completed(output))
        }
        _ = cancel_token.cancelled() => Ok(CommandRun::Interrupted),
    }
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
