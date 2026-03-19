use crate::config::{AgentCli, DEFAULT_CLAUDE_PERMISSION_MODE};
use anyhow::{bail, Context, Result};
use std::fmt::Write as FmtWrite;
use std::io::Write;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct AgentArgs<'a> {
    pub cli: AgentCli,
    pub permission_mode: &'a str,
    pub continue_session: bool,
    pub dangerously_skip_permissions: bool,
    pub timeout_secs: u64,
    pub project_dir: &'a std::path::Path,
    pub skip_git_repo_check: bool,
}

pub struct AgentResult {
    pub output: String,
    pub success: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
    display_name: &'static str,
}

pub async fn run_agent(
    prompt: &str,
    args: &AgentArgs<'_>,
    log_path: &std::path::Path,
    cancel_token: &CancellationToken,
) -> Result<AgentResult> {
    let duration = Duration::from_secs(args.timeout_secs);
    let spec = build_command_spec(args)?;

    let mut cmd = Command::new(spec.program);
    cmd.current_dir(args.project_dir);
    cmd.args(&spec.args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn {} CLI", spec.display_name))?;

    tokio::select! {
        result = run_agent_inner(&mut child, prompt, log_path) => result,
        _ = tokio::time::sleep(duration) => {
            let _ = child.kill().await;
            Ok(AgentResult {
                output: format!("Timeout: {} execution exceeded time limit", spec.display_name),
                success: false,
            })
        }
        _ = cancel_token.cancelled() => {
            let _ = child.kill().await;
            Ok(AgentResult {
                output: format!("Cancelled: {} execution was interrupted", spec.display_name),
                success: false,
            })
        }
    }
}

fn build_command_spec(args: &AgentArgs<'_>) -> Result<CommandSpec> {
    match args.cli {
        AgentCli::Claude => Ok(build_claude_command_spec(args)),
        AgentCli::Codex => build_codex_command_spec(args),
    }
}

fn build_claude_command_spec(args: &AgentArgs<'_>) -> CommandSpec {
    let mut spec = CommandSpec {
        program: "claude",
        args: vec![
            "--permission-mode".to_string(),
            args.permission_mode.to_string(),
            "--print".to_string(),
        ],
        display_name: "Claude",
    };

    if args.dangerously_skip_permissions {
        spec.args.push("--dangerously-skip-permissions".to_string());
    }
    if args.continue_session {
        spec.args.retain(|arg| arg != "--print");
        spec.args.push("--continue".to_string());
    }

    spec
}

fn build_codex_command_spec(args: &AgentArgs<'_>) -> Result<CommandSpec> {
    if args.continue_session {
        bail!("Codex does not support --continue-session");
    }
    if args.permission_mode != DEFAULT_CLAUDE_PERMISSION_MODE {
        bail!("Codex does not support --permission-mode");
    }

    let mut spec = CommandSpec {
        program: "codex",
        args: vec!["exec".to_string()],
        display_name: "Codex",
    };

    if args.dangerously_skip_permissions {
        spec.args
            .push("--dangerously-bypass-approvals-and-sandbox".to_string());
    } else {
        spec.args.push("--full-auto".to_string());
    }

    if args.skip_git_repo_check {
        spec.args.push("--skip-git-repo-check".to_string());
    }

    Ok(spec)
}

async fn run_agent_inner(
    child: &mut tokio::process::Child,
    prompt: &str,
    log_path: &std::path::Path,
) -> Result<AgentResult> {
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.shutdown().await?;
    }

    let stdout = child.stdout.take().context("Failed to capture stdout")?;
    let stderr = child.stderr.take().context("Failed to capture stderr")?;

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

    Ok(AgentResult {
        output,
        success: status.success(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn agent_args(cli: AgentCli) -> AgentArgs<'static> {
        AgentArgs {
            cli,
            permission_mode: DEFAULT_CLAUDE_PERMISSION_MODE,
            continue_session: false,
            dangerously_skip_permissions: false,
            timeout_secs: 1800,
            project_dir: Path::new("."),
            skip_git_repo_check: false,
        }
    }

    #[test]
    fn claude_command_defaults_to_print_mode() {
        let spec = build_command_spec(&agent_args(AgentCli::Claude)).unwrap();
        assert_eq!(spec.program, "claude");
        assert_eq!(
            spec.args,
            vec![
                "--permission-mode".to_string(),
                DEFAULT_CLAUDE_PERMISSION_MODE.to_string(),
                "--print".to_string(),
            ]
        );
    }

    #[test]
    fn claude_continue_session_replaces_print_mode() {
        let mut args = agent_args(AgentCli::Claude);
        args.continue_session = true;

        let spec = build_command_spec(&args).unwrap();

        assert_eq!(
            spec.args,
            vec![
                "--permission-mode".to_string(),
                DEFAULT_CLAUDE_PERMISSION_MODE.to_string(),
                "--continue".to_string(),
            ]
        );
    }

    #[test]
    fn claude_supports_dangerous_skip_permissions() {
        let mut args = agent_args(AgentCli::Claude);
        args.dangerously_skip_permissions = true;

        let spec = build_command_spec(&args).unwrap();

        assert_eq!(
            spec.args,
            vec![
                "--permission-mode".to_string(),
                DEFAULT_CLAUDE_PERMISSION_MODE.to_string(),
                "--print".to_string(),
                "--dangerously-skip-permissions".to_string(),
            ]
        );
    }

    #[test]
    fn codex_defaults_to_exec_and_full_auto() {
        let spec = build_command_spec(&agent_args(AgentCli::Codex)).unwrap();
        assert_eq!(spec.program, "codex");
        assert_eq!(
            spec.args,
            vec!["exec".to_string(), "--full-auto".to_string()]
        );
    }

    #[test]
    fn codex_supports_dangerous_skip_permissions() {
        let mut args = agent_args(AgentCli::Codex);
        args.dangerously_skip_permissions = true;

        let spec = build_command_spec(&args).unwrap();

        assert_eq!(
            spec.args,
            vec![
                "exec".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
            ]
        );
    }

    #[test]
    fn codex_can_skip_git_repo_check() {
        let mut args = agent_args(AgentCli::Codex);
        args.skip_git_repo_check = true;

        let spec = build_command_spec(&args).unwrap();

        assert_eq!(
            spec.args,
            vec![
                "exec".to_string(),
                "--full-auto".to_string(),
                "--skip-git-repo-check".to_string(),
            ]
        );
    }

    #[test]
    fn codex_rejects_continue_session() {
        let mut args = agent_args(AgentCli::Codex);
        args.continue_session = true;

        let err = build_command_spec(&args).unwrap_err().to_string();
        assert!(err.contains("--continue-session"));
    }

    #[test]
    fn codex_rejects_permission_mode() {
        let mut args = agent_args(AgentCli::Codex);
        args.permission_mode = "plan";

        let err = build_command_spec(&args).unwrap_err().to_string();
        assert!(err.contains("--permission-mode"));
    }
}
