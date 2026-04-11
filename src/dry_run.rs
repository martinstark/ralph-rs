use crate::{
    config::Args,
    git::{self, CommandRun},
    output,
    prd::Prd,
    subprocess,
};
use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub async fn run(args: &Args, prd: &Prd, cancel_token: &CancellationToken) -> Result<bool> {
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
    let git_status = match git::get_git_status_with_shutdown(cancel_token).await? {
        CommandRun::Completed(status) => status,
        CommandRun::Interrupted => return Ok(true),
    };
    if let Some(status) = git_status {
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

    if cancel_token.is_cancelled() {
        return Ok(true);
    }

    output::header("Verification Commands");
    let mut all_passed = true;
    for cmd in &prd.verification.commands {
        let success = match run_verification_command(&cmd.command, cancel_token).await? {
            Some(success) => success,
            None => return Ok(true),
        };

        if success {
            output::success(&format!("{}: PASS", cmd.name));
        } else {
            output::error(&format!("{}: FAIL", cmd.name));
            all_passed = false;
        }
    }
    println!();

    if cancel_token.is_cancelled() {
        return Ok(true);
    }

    output::separator();
    if all_passed {
        output::success("Dry run complete - all verifications passed");
    } else {
        output::warn("Dry run complete - some verifications failed");
    }
    output::separator();

    Ok(false)
}

async fn run_verification_command(
    command: &str,
    cancel_token: &CancellationToken,
) -> Result<Option<bool>> {
    if cancel_token.is_cancelled() {
        return Ok(None);
    }

    let mut child_command = Command::new("sh");
    child_command.arg("-c").arg(command);
    child_command.stdout(Stdio::null());
    child_command.stderr(Stdio::null());
    subprocess::configure_command(&mut child_command);

    let mut child = child_command
        .spawn()
        .with_context(|| format!("Failed to run verification command `{command}`"))?;
    let mut tracked_child = subprocess::track_child(&child);

    tokio::select! {
        status = child.wait() => {
            let status = status.with_context(|| format!("Failed to wait for verification command `{command}`"))?;
            subprocess::finish_child(&mut tracked_child);
            Ok(Some(status.success()))
        }
        _ = cancel_token.cancelled() => {
            subprocess::cleanup_child_process(
                &mut child,
                &mut tracked_child,
                "after a dry-run verification command was cancelled",
            )
            .await?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Args,
        prd::{Completion, Project, Verification, VerifyCommand},
    };
    use clap::Parser;
    use tempfile::TempDir;

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use tokio::time::{sleep, Duration};

        const ESRCH: i32 = 3;

        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }

        fn args() -> Args {
            Args::try_parse_from(["ralph", "--dry-run"]).unwrap()
        }

        fn prd_with_command(command: String) -> Prd {
            Prd {
                project: Project {
                    name: "test".to_string(),
                    description: "desc".to_string(),
                    repository: None,
                },
                verification: Verification {
                    commands: vec![VerifyCommand {
                        name: "verify".to_string(),
                        command,
                        description: "desc".to_string(),
                    }],
                    run_after_each_feature: true,
                },
                features: Vec::new(),
                completion: Completion {
                    all_features_complete: true,
                    all_verifications_passing: true,
                },
            }
        }

        fn write_script(dir: &TempDir, body: &str) -> PathBuf {
            let path = dir.path().join("verify.sh");
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).unwrap();
            path
        }

        fn read_pid(path: &Path) -> i32 {
            fs::read_to_string(path).unwrap().trim().parse().unwrap()
        }

        async fn wait_for_file(path: &Path) {
            for _ in 0..100 {
                if path.exists() {
                    return;
                }
                sleep(Duration::from_millis(20)).await;
            }
            panic!("timed out waiting for {}", path.display());
        }

        async fn wait_for_process_exit(pid: i32) {
            for _ in 0..100 {
                if !process_is_alive(pid) {
                    return;
                }
                sleep(Duration::from_millis(20)).await;
            }
            panic!("process {pid} is still alive");
        }

        fn process_is_alive(pid: i32) -> bool {
            let result = unsafe { kill(pid, 0) };
            if result == 0 {
                true
            } else {
                !matches!(std::io::Error::last_os_error().raw_os_error(), Some(ESRCH))
            }
        }

        #[tokio::test]
        async fn dry_run_cleans_up_verification_process_group_on_cancellation() {
            let _guard = crate::subprocess::test_process_lock().lock().unwrap();
            let temp_dir = TempDir::new().unwrap();
            let parent_pid_path = temp_dir.path().join("parent.pid");
            let child_pid_path = temp_dir.path().join("child.pid");
            let script = write_script(
                &temp_dir,
                &format!(
                    "echo $$ > {parent}\n\
                     trap 'exit 0' INT\n\
                     sh -c 'trap \"\" INT; echo $$ > {child}; sleep 30' &\n\
                     wait",
                    parent = parent_pid_path.display(),
                    child = child_pid_path.display(),
                ),
            );
            let prd = prd_with_command(script.display().to_string());
            let cancel_token = CancellationToken::new();

            let task = tokio::spawn({
                let cancel_token = cancel_token.clone();
                async move { run(&args(), &prd, &cancel_token).await.unwrap() }
            });

            wait_for_file(&child_pid_path).await;
            cancel_token.cancel();

            assert!(task.await.unwrap());
            wait_for_process_exit(read_pid(&parent_pid_path)).await;
            wait_for_process_exit(read_pid(&child_pid_path)).await;
        }
    }
}
