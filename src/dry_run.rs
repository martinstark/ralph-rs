use crate::{config::Args, git, output, prd::Prd};
use anyhow::Result;
use std::process::Command;

pub fn run(args: &Args, prd: &Prd) -> Result<()> {
    output::section("Dry Run Mode");

    output::header("PRD Summary");
    output::log(&format!("Project: {}", prd.project.name));
    output::log(&format!("PRD file: {}", args.prd.display()));
    println!();

    let counts = prd.status_counts();
    let total = prd.features.len();
    output::header("Feature Status");
    output::log(&format!("Total features: {total}"));
    output::log(&format!("  Pending:     {}", counts.pending));
    output::log(&format!("  In-progress: {}", counts.in_progress));
    output::log(&format!("  Complete:    {}", counts.complete));
    output::log(&format!("  Blocked:     {}", counts.blocked));
    println!();

    output::header("Git Status");
    if let Some(status) = git::get_git_status() {
        output::log(&format!("Branch: {}", status.branch));
        output::log(&format!(
            "Uncommitted changes: {}",
            status.uncommitted_changes
        ));
        if status.uncommitted_changes > 0 {
            output::dim("  (Uncommitted changes are informational only)");
        }
    } else {
        output::warn("Not a git repository");
    }
    println!();

    output::header("Verification Commands");
    let mut all_passed = true;
    for cmd in &prd.verification.commands {
        let result = Command::new("sh").args(["-c", &cmd.command]).output();

        match result {
            Ok(output) if output.status.success() => {
                output::success(&format!("{}: PASS", cmd.name));
            }
            Ok(_) => {
                output::error(&format!("{}: FAIL", cmd.name));
                all_passed = false;
            }
            Err(e) => {
                output::error(&format!("{}: ERROR ({})", cmd.name, e));
                all_passed = false;
            }
        }
    }
    println!();

    output::separator();
    if all_passed {
        output::success("Dry run complete - all verifications passed");
    } else {
        output::warn("Dry run complete - some verifications failed");
    }
    output::separator();

    Ok(())
}
