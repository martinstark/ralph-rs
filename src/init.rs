use crate::{
    git::{self, CommandRun},
    output,
    prd::Prd,
};
use anyhow::Result;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub async fn run_init_phase(
    prd: &Prd,
    prd_path: &Path,
    progress_path: &Path,
    cancel_token: &CancellationToken,
) -> Result<bool> {
    output::section("Phase 1: Initialization");

    output::log("Step 1: Checking git status...");
    let git_status = match git::get_git_status_with_shutdown(cancel_token).await? {
        CommandRun::Completed(status) => status,
        CommandRun::Interrupted => return Ok(true),
    };
    let git_repo_available = git_status.is_some();

    match git_status {
        Some(status) if status.uncommitted_changes > 0 => {
            output::warn(&format!(
                "Branch: {} ({} uncommitted changes)",
                status.branch, status.uncommitted_changes
            ));
        }
        Some(status) => output::success(&format!("Branch: {} (clean)", status.branch)),
        None => output::warn("Not a git repository - git features disabled"),
    }

    if cancel_token.is_cancelled() {
        return Ok(true);
    }

    output::log("Step 2: Reading PRD...");
    let counts = prd.status_counts();
    let total = prd.features.len();
    output::success(&format!(
        "PRD: {total} features ({} complete, {} in-progress, {} pending, {} blocked)",
        counts.complete, counts.in_progress, counts.pending, counts.blocked
    ));
    output::log(&format!("PRD file: {}", prd_path.display()));

    if cancel_token.is_cancelled() {
        return Ok(true);
    }

    output::log("Step 3: Checking progress file...");
    if progress_path.exists() {
        let content = std::fs::read_to_string(progress_path).unwrap_or_default();
        let sessions = content.matches("## Session").count();
        output::success(&format!("Progress: {sessions} previous sessions recorded"));
    } else {
        output::dim("Progress file will be created");
    }
    output::log(&format!("Progress file: {}", progress_path.display()));

    if cancel_token.is_cancelled() {
        return Ok(true);
    }

    if git_repo_available {
        output::log("Step 4: Recent git history...");
        println!();
        let commits = match git::recent_commits_with_shutdown(5, cancel_token).await? {
            CommandRun::Completed(commits) => commits,
            CommandRun::Interrupted => return Ok(true),
        };
        for commit in commits {
            println!("  {commit}");
        }
        println!();
    }

    if cancel_token.is_cancelled() {
        return Ok(true);
    }

    output::separator();
    output::success("Initialization complete - ready for Ralph iteration");
    output::separator();
    println!();

    Ok(false)
}
