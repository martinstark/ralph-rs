use anyhow::{Context, Result};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct ClaudeArgs<'a> {
    pub permission_mode: String,
    pub continue_session: bool,
    pub dangerously_skip_permissions: bool,
    pub timeout_secs: u64,
    pub project_dir: &'a std::path::Path,
}

pub struct ClaudeResult {
    pub output: String,
    pub success: bool,
}

pub async fn run_claude(
    prompt: &str,
    args: &ClaudeArgs<'_>,
    log_path: &std::path::Path,
    cancel_token: &CancellationToken,
) -> Result<ClaudeResult> {
    let duration = Duration::from_secs(args.timeout_secs);

    let mut cmd = Command::new("claude");
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
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().context("Failed to spawn claude CLI")?;

    tokio::select! {
        result = run_claude_inner(&mut child, prompt, log_path) => result,
        _ = tokio::time::sleep(duration) => {
            let _ = child.kill().await;
            Ok(ClaudeResult {
                output: "Timeout: Claude execution exceeded time limit".to_string(),
                success: false,
            })
        }
        _ = cancel_token.cancelled() => {
            let _ = child.kill().await;
            Ok(ClaudeResult {
                output: "Cancelled: Claude execution was interrupted".to_string(),
                success: false,
            })
        }
    }
}

async fn run_claude_inner(child: &mut tokio::process::Child, prompt: &str, log_path: &std::path::Path) -> Result<ClaudeResult> {
    // Write prompt to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await?;
    }

    let stdout = child.stdout.take().context("Failed to capture stdout")?;
    let stderr = child.stderr.take().context("Failed to capture stderr")?;

    let mut log_file = std::fs::File::create(log_path)
        .context("Failed to create log file")?;

    let mut output = String::new();

    // Stream stdout
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
                match line {
                    Ok(Some(line)) => {
                        println!("{line}");
                        writeln!(log_file, "{line}")?;
                        let _ = writeln!(output, "{line}");
                    }
                    Ok(None) => stdout_done = true,
                    Err(e) => {
                        eprintln!("Error reading stdout: {e}");
                        stdout_done = true;
                    }
                }
            }
            line = stderr_reader.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(line)) => {
                        eprintln!("{line}");
                        writeln!(log_file, "[stderr] {line}")?;
                        let _ = writeln!(output, "{line}");
                    }
                    Ok(None) => stderr_done = true,
                    Err(e) => {
                        eprintln!("Error reading stderr: {e}");
                        stderr_done = true;
                    }
                }
            }
        }
    }

    let status = child.wait().await?;

    Ok(ClaudeResult {
        output,
        success: status.success(),
    })
}
