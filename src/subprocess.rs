use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};

const CLEANUP_GRACE_PERIOD: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
struct ActiveProcess {
    pid: u32,
    process_group: Option<u32>,
}

pub(crate) struct TrackedChild {
    registration_id: Option<u64>,
    process_group: Option<u32>,
}

static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_PROCESSES: OnceLock<Mutex<HashMap<u64, ActiveProcess>>> = OnceLock::new();
#[cfg(test)]
static TEST_PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn configure_command(command: &mut Command) {
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
}

pub(crate) fn track_child(child: &Child) -> TrackedChild {
    let pid = child.id();
    let process_group = capture_process_group(child);
    let registration_id = pid.map(|pid| {
        let registration_id = NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed);
        active_processes()
            .lock()
            .unwrap()
            .insert(registration_id, ActiveProcess { pid, process_group });
        registration_id
    });

    TrackedChild {
        registration_id,
        process_group,
    }
}

pub(crate) fn finish_child(tracked_child: &mut TrackedChild) {
    if let Some(registration_id) = tracked_child.registration_id.take() {
        active_processes().lock().unwrap().remove(&registration_id);
    }
}

pub(crate) fn force_terminate_tracked_processes() -> Result<()> {
    let processes: Vec<ActiveProcess> = active_processes()
        .lock()
        .unwrap()
        .values()
        .copied()
        .collect();

    #[cfg(unix)]
    {
        force_terminate_unix_processes(&processes)
    }

    #[cfg(not(unix))]
    {
        let _ = processes;
        Ok(())
    }
}

fn active_processes() -> &'static Mutex<HashMap<u64, ActiveProcess>> {
    ACTIVE_PROCESSES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub(crate) fn test_process_lock() -> &'static Mutex<()> {
    TEST_PROCESS_LOCK.get_or_init(|| Mutex::new(()))
}

fn capture_process_group(child: &Child) -> Option<u32> {
    #[cfg(unix)]
    {
        child.id()
    }

    #[cfg(not(unix))]
    {
        let _ = child;
        None
    }
}

pub(crate) async fn cleanup_child_process(
    child: &mut Child,
    tracked_child: &mut TrackedChild,
    reason: &str,
) -> Result<()> {
    let result = {
        #[cfg(unix)]
        if let Some(process_group) = tracked_child.process_group {
            cleanup_unix_process_group(child, process_group, reason).await
        } else {
            cleanup_direct_child(child, reason).await
        }

        #[cfg(not(unix))]
        cleanup_direct_child(child, reason).await
    };

    if result.is_ok() {
        finish_child(tracked_child);
    }
    result
}

#[cfg(unix)]
async fn cleanup_unix_process_group(
    child: &mut Child,
    process_group: u32,
    reason: &str,
) -> Result<()> {
    let mut leader_reaped = try_reap_child(child, reason)?;

    if !process_group_has_members(process_group)
        .with_context(|| format!("Failed to inspect process group {reason}"))?
    {
        ensure_leader_cleanup(child, leader_reaped, reason).await?;
        return Ok(());
    }

    signal_process_group(process_group, SIGINT)
        .with_context(|| format!("Failed to interrupt process group {reason}"))?;

    let deadline = tokio::time::Instant::now() + CLEANUP_GRACE_PERIOD;
    loop {
        if !leader_reaped {
            leader_reaped = try_reap_child(child, reason)?;
        }

        if !process_group_has_members(process_group)
            .with_context(|| format!("Failed to inspect process group {reason}"))?
        {
            ensure_leader_cleanup(child, leader_reaped, reason).await?;
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            break;
        }

        sleep(Duration::from_millis(25)).await;
    }

    signal_process_group(process_group, SIGKILL)
        .with_context(|| format!("Failed to force-kill process group {reason}"))?;

    ensure_leader_cleanup(child, leader_reaped, reason).await?;

    wait_for_process_group_exit(process_group, reason).await
}

#[cfg(unix)]
fn try_reap_child(child: &mut Child, reason: &str) -> Result<bool> {
    Ok(child
        .try_wait()
        .with_context(|| format!("Failed to inspect process {reason}"))?
        .is_some())
}

#[cfg(unix)]
async fn ensure_leader_cleanup(child: &mut Child, leader_reaped: bool, reason: &str) -> Result<()> {
    if leader_reaped {
        return Ok(());
    }

    if try_reap_child(child, reason)? {
        return Ok(());
    }

    kill_child_process(child, reason).await
}

async fn cleanup_direct_child(child: &mut Child, reason: &str) -> Result<()> {
    if child
        .try_wait()
        .with_context(|| format!("Failed to inspect process {reason}"))?
        .is_some()
    {
        return Ok(());
    }

    kill_child_process(child, reason).await
}

async fn kill_child_process(child: &mut Child, reason: &str) -> Result<()> {
    child
        .kill()
        .await
        .or_else(|error| {
            child
                .try_wait()
                .with_context(|| format!("Failed to inspect process {reason}"))
                .and_then(|status| match status {
                    Some(_) => Ok(()),
                    None => Err(error.into()),
                })
        })
        .with_context(|| format!("Failed to kill process {reason}"))?;
    child
        .wait()
        .await
        .with_context(|| format!("Failed to reap process {reason}"))?;
    Ok(())
}

#[cfg(unix)]
const SIGINT: i32 = 2;
#[cfg(unix)]
const SIGKILL: i32 = 9;
#[cfg(unix)]
const ESRCH: i32 = 3;

#[cfg(unix)]
fn signal_process_group(process_group: u32, signal: i32) -> std::io::Result<()> {
    let error = match send_signal_to_process_group(process_group, signal) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    if matches!(error.raw_os_error(), Some(ESRCH)) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn process_group_has_members(process_group: u32) -> std::io::Result<bool> {
    match send_signal_to_process_group(process_group, 0) {
        Ok(()) => Ok(true),
        Err(error) if matches!(error.raw_os_error(), Some(ESRCH)) => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
async fn wait_for_process_group_exit(process_group: u32, reason: &str) -> Result<()> {
    let deadline = tokio::time::Instant::now() + CLEANUP_GRACE_PERIOD;
    loop {
        if !process_group_has_members(process_group)
            .with_context(|| format!("Failed to inspect process group {reason}"))?
        {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            bail!("Process group still had live descendants {reason}");
        }

        sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(unix)]
fn send_signal_to_process_group(process_group: u32, signal: i32) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};

    let process_group = i32::try_from(process_group).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("Process group {process_group} does not fit in pid_t"),
        )
    })?;

    let result = unsafe { kill(-process_group, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(Error::last_os_error())
    }
}

#[cfg(unix)]
fn force_terminate_unix_processes(processes: &[ActiveProcess]) -> Result<()> {
    use std::collections::HashSet;

    let mut seen_groups = HashSet::new();
    let mut seen_pids = HashSet::new();
    let mut errors = Vec::new();

    for process in processes {
        if let Some(process_group) = process.process_group {
            if seen_groups.insert(process_group) {
                if let Err(error) = signal_process_group(process_group, SIGKILL) {
                    errors.push(format!("process group {process_group}: {error}"));
                }
            }
        } else if seen_pids.insert(process.pid) {
            if let Err(error) = signal_process(process.pid, SIGKILL) {
                errors.push(format!("process {}: {error}", process.pid));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        bail!(
            "Failed to force-kill active subprocesses: {}",
            errors.join("; ")
        )
    }
}

#[cfg(unix)]
fn signal_process(pid: u32, signal: i32) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};

    let pid = i32::try_from(pid).map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("Process {pid} does not fit in pid_t"),
        )
    })?;

    let result = unsafe { kill(pid, signal) };
    if result == 0 {
        Ok(())
    } else {
        let error = Error::last_os_error();
        if matches!(error.raw_os_error(), Some(ESRCH)) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};

        fn write_script(dir: &TempDir, body: &str) -> PathBuf {
            let path = dir.path().join("process.sh");
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
                let result = unsafe { kill(pid, 0) };
                if result != 0 {
                    let error = std::io::Error::last_os_error();
                    if matches!(error.raw_os_error(), Some(ESRCH)) {
                        return;
                    }
                }
                sleep(Duration::from_millis(20)).await;
            }
            panic!("process {pid} is still alive");
        }

        #[tokio::test]
        async fn force_terminate_tracked_processes_kills_registered_process_groups() {
            let _guard = test_process_lock().lock().unwrap();
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

            let mut command = Command::new(&script);
            configure_command(&mut command);
            let mut child = command.spawn().unwrap();
            let mut tracked_child = track_child(&child);

            wait_for_file(&child_pid_path).await;
            force_terminate_tracked_processes().unwrap();

            child.wait().await.unwrap();
            finish_child(&mut tracked_child);
            wait_for_process_exit(read_pid(&parent_pid_path)).await;
            wait_for_process_exit(read_pid(&child_pid_path)).await;
        }
    }
}
