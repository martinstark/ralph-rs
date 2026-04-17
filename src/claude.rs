use crate::subprocess;
use anyhow::{Context, Result};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::process::{ExitStatus, Stdio};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

const INTERRUPTED_MESSAGE: &str = "Cancelled: Claude execution was interrupted";
const TIMED_OUT_MESSAGE: &str = "Timeout: Claude execution exceeded time limit";

pub struct ClaudeArgs<'a> {
    pub permission_mode: String,
    pub continue_session: bool,
    pub dangerously_skip_permissions: bool,
    pub timeout_secs: u64,
    pub project_dir: &'a std::path::Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeOutcome {
    Success,
    Failure,
    TimedOut,
    Cancelled,
}

pub struct ClaudeResult {
    pub output: String,
    pub outcome: ClaudeOutcome,
}

impl ClaudeResult {
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self.outcome, ClaudeOutcome::Success)
    }
}

enum ClaudeExecution {
    Completed(Result<ClaudeResult>),
    TimedOut,
    Cancelled,
}

pub async fn run_claude(
    prompt: &str,
    args: &ClaudeArgs<'_>,
    log_path: &std::path::Path,
    cancel_token: &CancellationToken,
) -> Result<ClaudeResult> {
    run_claude_command(
        std::path::Path::new("claude"),
        prompt,
        args,
        log_path,
        cancel_token,
    )
    .await
}

async fn run_claude_command(
    program: &std::path::Path,
    prompt: &str,
    args: &ClaudeArgs<'_>,
    log_path: &std::path::Path,
    cancel_token: &CancellationToken,
) -> Result<ClaudeResult> {
    if cancel_token.is_cancelled() {
        return Ok(cancelled_result());
    }

    let mut child = build_claude_command(program, args)
        .spawn()
        .context("Failed to spawn claude CLI")?;
    let mut tracked_child = subprocess::track_child(&child);

    let execution = tokio::select! {
        result = run_claude_inner(&mut child, prompt, log_path, cancel_token) => ClaudeExecution::Completed(result),
        _ = tokio::time::sleep(Duration::from_secs(args.timeout_secs)) => ClaudeExecution::TimedOut,
        _ = cancel_token.cancelled() => ClaudeExecution::Cancelled,
    };

    match execution {
        ClaudeExecution::Completed(Ok(result)) => {
            subprocess::finish_child(&mut tracked_child);
            Ok(result)
        }
        ClaudeExecution::Completed(Err(error)) if cancel_token.is_cancelled() => {
            subprocess::cleanup_child_process(
                &mut child,
                &mut tracked_child,
                "after Claude execution failed during shutdown",
            )
            .await
            .with_context(|| {
                format!(
                    "Claude cleanup during shutdown failed after Claude returned an error: {error:#}"
                )
            })?;
            Ok(cancelled_result())
        }
        ClaudeExecution::Completed(Err(error)) => {
            subprocess::cleanup_child_process(
                &mut child,
                &mut tracked_child,
                "after Claude execution failed",
            )
            .await?;
            Err(error)
        }
        ClaudeExecution::TimedOut => {
            subprocess::cleanup_child_process(
                &mut child,
                &mut tracked_child,
                "after Claude timed out",
            )
            .await?;
            Ok(timed_out_result())
        }
        ClaudeExecution::Cancelled => interrupted_after_cleanup(
            subprocess::cleanup_child_process(
                &mut child,
                &mut tracked_child,
                "after Claude was cancelled",
            )
            .await,
        ),
    }
}

fn build_claude_command(program: &std::path::Path, args: &ClaudeArgs<'_>) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(args.project_dir);
    cmd.arg("--permission-mode").arg(&args.permission_mode);
    if args.dangerously_skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }
    if args.continue_session {
        cmd.arg("--continue");
    } else {
        cmd.arg("--print");
    }
    subprocess::configure_command(&mut cmd);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd
}

async fn run_claude_inner(
    child: &mut Child,
    prompt: &str,
    log_path: &std::path::Path,
    cancel_token: &CancellationToken,
) -> Result<ClaudeResult> {
    let mut stdin = child
        .stdin
        .take()
        .context("Failed to capture Claude stdin")?;
    stdin
        .write_all(prompt.as_bytes())
        .await
        .context("Failed to write Claude prompt to stdin")?;
    stdin
        .shutdown()
        .await
        .context("Failed to close Claude stdin")?;
    // Child::wait() only auto-closes stdin if the handle is still owned by Child.
    // We took it above, so drop our handle explicitly to deliver EOF to Claude.
    drop(stdin);

    let stdout = child
        .stdout
        .take()
        .context("Failed to capture Claude stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("Failed to capture Claude stderr")?;

    let mut log_file = std::fs::File::create(log_path).context("Failed to create log file")?;
    let mut output = String::new();
    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();
    let mut stdout_done = false;
    let mut stderr_done = false;

    loop {
        if stdout_done && stderr_done {
            break;
        }

        tokio::select! {
            line = stdout_reader.next_line(), if !stdout_done => {
                match line.context("Failed to read Claude stdout")? {
                    Some(line) => {
                        println!("{line}");
                        writeln!(log_file, "{line}")
                            .context("Failed to write Claude stdout to log file")?;
                        let _ = writeln!(output, "{line}");
                    }
                    None => stdout_done = true,
                }
            }
            line = stderr_reader.next_line(), if !stderr_done => {
                match line.context("Failed to read Claude stderr")? {
                    Some(line) => {
                        eprintln!("{line}");
                        writeln!(log_file, "[stderr] {line}")
                            .context("Failed to write Claude stderr to log file")?;
                        let _ = writeln!(output, "{line}");
                    }
                    None => stderr_done = true,
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .context("Failed to wait for Claude process")?;

    Ok(ClaudeResult {
        output,
        outcome: classify_exit_status(status, cancel_token.is_cancelled()),
    })
}

fn cancelled_result() -> ClaudeResult {
    ClaudeResult {
        output: INTERRUPTED_MESSAGE.to_string(),
        outcome: ClaudeOutcome::Cancelled,
    }
}

fn interrupted_after_cleanup(cleanup_result: Result<()>) -> Result<ClaudeResult> {
    cleanup_result.context("Claude cleanup during shutdown failed")?;
    Ok(cancelled_result())
}

fn timed_out_result() -> ClaudeResult {
    ClaudeResult {
        output: TIMED_OUT_MESSAGE.to_string(),
        outcome: ClaudeOutcome::TimedOut,
    }
}

fn classify_exit_status(status: ExitStatus, shutdown_requested: bool) -> ClaudeOutcome {
    if shutdown_requested || exit_status_was_interrupted(&status) {
        ClaudeOutcome::Cancelled
    } else if status.success() {
        ClaudeOutcome::Success
    } else {
        ClaudeOutcome::Failure
    }
}

#[cfg(unix)]
fn exit_status_was_interrupted(status: &ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;

    matches!(status.signal(), Some(SIGINT)) || matches!(status.code(), Some(130))
}

#[cfg(windows)]
fn exit_status_was_interrupted(status: &ExitStatus) -> bool {
    matches!(status.code(), Some(130) | Some(-1073741510))
}

#[cfg(not(any(unix, windows)))]
fn exit_status_was_interrupted(status: &ExitStatus) -> bool {
    matches!(status.code(), Some(130))
}

#[cfg(unix)]
const SIGINT: i32 = 2;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::fs;
        use std::os::unix::{fs::PermissionsExt, process::ExitStatusExt};
        use std::path::{Path, PathBuf};
        use tokio::time::sleep;

        const ESRCH: i32 = 3;

        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }

        fn exit_status(code: i32) -> ExitStatus {
            ExitStatus::from_raw(code << 8)
        }

        fn signal_status(signal: i32) -> ExitStatus {
            ExitStatus::from_raw(signal)
        }

        fn claude_args<'a>(project_dir: &'a Path, timeout_secs: u64) -> ClaudeArgs<'a> {
            ClaudeArgs {
                permission_mode: "acceptEdits".to_string(),
                continue_session: false,
                dangerously_skip_permissions: false,
                timeout_secs,
                project_dir,
            }
        }

        fn write_script(dir: &TempDir, body: &str) -> PathBuf {
            let path = dir.path().join("fake-claude.sh");
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
            if matches!(process_state(pid), Some('Z')) {
                return false;
            }

            let result = unsafe { kill(pid, 0) };
            if result == 0 {
                true
            } else {
                !matches!(std::io::Error::last_os_error().raw_os_error(), Some(ESRCH))
            }
        }

        fn process_state(pid: i32) -> Option<char> {
            let path = format!("/proc/{pid}/stat");
            let content = fs::read_to_string(path).ok()?;
            content
                .rsplit_once(") ")
                .and_then(|(_, rest)| rest.chars().next())
        }

        #[test]
        fn build_claude_command_enables_kill_on_drop() {
            let command =
                build_claude_command(Path::new("claude"), &claude_args(Path::new("."), 1));
            assert!(command.get_kill_on_drop());
        }

        #[test]
        fn classify_exit_status_returns_success_for_zero_exit() {
            assert_eq!(
                classify_exit_status(exit_status(0), false),
                ClaudeOutcome::Success
            );
        }

        #[test]
        fn classify_exit_status_returns_failure_for_non_zero_exit() {
            assert_eq!(
                classify_exit_status(exit_status(1), false),
                ClaudeOutcome::Failure
            );
        }

        #[test]
        fn classify_exit_status_prefers_cancelled_when_shutdown_requested() {
            assert_eq!(
                classify_exit_status(exit_status(1), true),
                ClaudeOutcome::Cancelled
            );
        }

        #[test]
        fn classify_exit_status_maps_sigint_to_cancelled() {
            assert_eq!(
                classify_exit_status(signal_status(SIGINT), false),
                ClaudeOutcome::Cancelled
            );
        }

        #[test]
        fn classify_exit_status_maps_exit_code_130_to_cancelled() {
            assert_eq!(
                classify_exit_status(exit_status(130), false),
                ClaudeOutcome::Cancelled
            );
        }

        #[test]
        fn interrupted_after_cleanup_returns_cancelled_on_success() {
            assert_eq!(
                interrupted_after_cleanup(Ok(())).unwrap().outcome,
                ClaudeOutcome::Cancelled
            );
        }

        #[test]
        fn interrupted_after_cleanup_surfaces_cleanup_errors() {
            let error = interrupted_after_cleanup(Err(anyhow::anyhow!("cleanup failed")))
                .err()
                .unwrap();
            assert!(error
                .to_string()
                .contains("Claude cleanup during shutdown failed"));
        }

        #[tokio::test]
        async fn run_claude_command_cleans_up_process_group_on_cancellation() {
            let _guard = crate::subprocess::test_process_lock().lock().unwrap();
            let temp_dir = TempDir::new().unwrap();
            let parent_pid_path = temp_dir.path().join("parent.pid");
            let child_pid_path = temp_dir.path().join("child.pid");
            let script = write_script(
                &temp_dir,
                &format!(
                    "echo $$ > {parent}\n\
                     sleep 30 &\n\
                     child=$!\n\
                     echo \"$child\" > {child}\n\
                     wait \"$child\"",
                    parent = parent_pid_path.display(),
                    child = child_pid_path.display(),
                ),
            );
            let log_path = temp_dir.path().join("claude.log");
            let cancel_token = CancellationToken::new();

            let task = tokio::spawn({
                let cancel_token = cancel_token.clone();
                let project_dir = temp_dir.path().to_path_buf();
                async move {
                    let args = claude_args(&project_dir, 30);
                    run_claude_command(&script, "prompt", &args, &log_path, &cancel_token)
                        .await
                        .unwrap()
                }
            });

            wait_for_file(&child_pid_path).await;
            cancel_token.cancel();

            let result = task.await.unwrap();
            assert_eq!(result.outcome, ClaudeOutcome::Cancelled);
            wait_for_process_exit(read_pid(&parent_pid_path)).await;
            wait_for_process_exit(read_pid(&child_pid_path)).await;
        }

        #[tokio::test]
        async fn run_claude_command_cleans_up_descendants_after_leader_exits() {
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
            let log_path = temp_dir.path().join("claude.log");
            let cancel_token = CancellationToken::new();

            let task = tokio::spawn({
                let cancel_token = cancel_token.clone();
                let project_dir = temp_dir.path().to_path_buf();
                async move {
                    let args = claude_args(&project_dir, 30);
                    run_claude_command(&script, "prompt", &args, &log_path, &cancel_token)
                        .await
                        .unwrap()
                }
            });

            wait_for_file(&child_pid_path).await;
            cancel_token.cancel();

            let result = task.await.unwrap();
            assert_eq!(result.outcome, ClaudeOutcome::Cancelled);
            wait_for_process_exit(read_pid(&parent_pid_path)).await;
            wait_for_process_exit(read_pid(&child_pid_path)).await;
        }

        #[tokio::test]
        async fn run_claude_command_cleans_up_process_group_on_timeout() {
            let _guard = crate::subprocess::test_process_lock().lock().unwrap();
            let temp_dir = TempDir::new().unwrap();
            let parent_pid_path = temp_dir.path().join("parent.pid");
            let child_pid_path = temp_dir.path().join("child.pid");
            let script = write_script(
                &temp_dir,
                &format!(
                    "echo $$ > {parent}\n\
                     sleep 30 &\n\
                     child=$!\n\
                     echo \"$child\" > {child}\n\
                     wait \"$child\"",
                    parent = parent_pid_path.display(),
                    child = child_pid_path.display(),
                ),
            );
            let log_path = temp_dir.path().join("claude.log");
            let args = claude_args(temp_dir.path(), 1);
            let result = run_claude_command(
                &script,
                "prompt",
                &args,
                &log_path,
                &CancellationToken::new(),
            )
            .await
            .unwrap();

            assert_eq!(result.outcome, ClaudeOutcome::TimedOut);
            wait_for_file(&child_pid_path).await;
            wait_for_process_exit(read_pid(&parent_pid_path)).await;
            wait_for_process_exit(read_pid(&child_pid_path)).await;
        }

        #[tokio::test]
        async fn run_claude_command_drops_taken_stdin_to_deliver_eof() {
            let _guard = crate::subprocess::test_process_lock().lock().unwrap();
            let temp_dir = TempDir::new().unwrap();
            let stdin_path = temp_dir.path().join("stdin.txt");
            let script = write_script(
                &temp_dir,
                &format!(
                    "echo pre-stdin\n\
                     cat > {stdin}\n\
                     echo post-stdin\n\
                     echo '<promise>COMPLETE</promise>'",
                    stdin = stdin_path.display(),
                ),
            );
            let log_path = temp_dir.path().join("claude.log");
            let args = claude_args(temp_dir.path(), 5);
            let result = run_claude_command(
                &script,
                "prompt",
                &args,
                &log_path,
                &CancellationToken::new(),
            )
            .await
            .unwrap();

            assert_eq!(result.outcome, ClaudeOutcome::Success);
            assert!(result.output.contains("pre-stdin"));
            assert!(result.output.contains("post-stdin"));
            assert!(result.output.contains("<promise>COMPLETE</promise>"));
            assert_eq!(fs::read_to_string(&stdin_path).unwrap(), "prompt");
            assert!(fs::read_to_string(&log_path).unwrap().contains("post-stdin"));
        }
    }
}
