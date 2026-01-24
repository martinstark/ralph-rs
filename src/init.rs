use crate::{git, output, prd::Prd};
use anyhow::Result;
use std::path::Path;

pub fn run_init_phase(prd: &Prd, prd_path: &Path, progress_path: &Path) -> Result<()> {
    output::section("Phase 1: Initialization");

    // Step 1: Verify git repository
    output::log("Step 1: Checking git status...");
    match git::get_git_status() {
        Some(status) if status.uncommitted_changes > 0 => {
            output::warn(&format!(
                "Branch: {} ({} uncommitted changes)",
                status.branch, status.uncommitted_changes
            ));
        }
        Some(status) => output::success(&format!("Branch: {} (clean)", status.branch)),
        None => output::warn("Not a git repository - git features disabled"),
    }

    // Step 2: PRD summary
    output::log("Step 2: Reading PRD...");
    let c = prd.status_counts();
    let total = prd.features.len();
    output::success(&format!(
        "PRD: {total} features ({} complete, {} in-progress, {} pending, {} blocked)",
        c.complete, c.in_progress, c.pending, c.blocked
    ));
    output::log(&format!("PRD file: {}", prd_path.display()));

    // Step 3: Progress file
    output::log("Step 3: Checking progress file...");
    if progress_path.exists() {
        let content = std::fs::read_to_string(progress_path).unwrap_or_default();
        let sessions = content.matches("## Session").count();
        output::success(&format!(
            "Progress: {sessions} previous sessions recorded"
        ));
    } else {
        output::dim("Progress file will be created");
    }
    output::log(&format!("Progress file: {}", progress_path.display()));

    // Step 4: Recent git history
    if git::is_git_repo() {
        output::log("Step 4: Recent git history...");
        println!();
        if let Ok(commits) = git::recent_commits(5) {
            for commit in commits {
                println!("  {commit}");
            }
        }
        println!();
    }

    output::separator();
    output::success("Initialization complete - ready for Ralph iteration");
    output::separator();
    println!();

    Ok(())
}
