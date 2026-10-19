use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

static RUNNING: AtomicBool = AtomicBool::new(false);

const TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug)]
pub(crate) struct Busy;

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("テストを実行中です")
    }
}

impl std::error::Error for Busy {}

#[derive(Debug, Serialize)]
pub(crate) struct RunResult {
    pub name: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub output_tail: String,
}

/// Runs one generated test from the repository root. The atomic guard is deliberately
/// process-wide: the HTTP handler can return 409 instead of queueing expensive Cargo jobs.
pub(crate) async fn run(name: &str) -> Result<RunResult> {
    if !RUNNING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        return Err(anyhow!(Busy));
    }

    // The Drop implementation also releases the lock when an HTTP request is
    // cancelled while Cargo is still running.
    let _guard = RunGuard;
    run_inner(name).await
}

struct RunGuard;

impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

async fn run_inner(name: &str) -> Result<RunResult> {
    if !crate::valid_name(name) {
        return Err(anyhow!("テスト名が正しくありません"));
    }

    let started = Instant::now();
    let root = repo_root();
    let mut command = Command::new("cargo");
    command
        .args(["test", "-p", "libsignal-protocol", "--test"])
        .arg(format!("viz_{name}"))
        .current_dir(root);
    let (passed, exit_code, output_tail) = execute(command, TIMEOUT).await?;
    Ok(RunResult {
        name: name.to_owned(),
        passed,
        exit_code,
        duration_ms: started.elapsed().as_millis(),
        output_tail,
    })
}

async fn execute(mut command: Command, timeout: Duration) -> Result<(bool, Option<i32>, String)> {
    let mut child = command
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let tail = Mutex::new(VecDeque::with_capacity(60));
    // Keep the readers in this future: timeout or cancellation drops their pipe
    // handles too, even if a Cargo descendant inherited an output handle.
    let finished = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            child.wait(),
            capture_lines(stdout, &tail),
            capture_lines(stderr, &tail),
        )
    })
    .await;
    let (passed, code) = match finished {
        Ok(result) => {
            let (status, (), ()) = result?;
            (status.success(), status.code())
        }
        Err(_) => {
            if child.try_wait()?.is_none() {
                child.kill().await?;
            }
            child.wait().await?;
            push_line(&tail, "cargo test は15分でタイムアウトしました".to_owned());
            (false, None)
        }
    };
    let output = tail
        .into_inner()
        .expect("tail lock")
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n");
    Ok((passed, code, output))
}

/// Formats generated source when rustfmt is installed. The server remains usable
/// in minimal installations where rustfmt was not installed with the toolchain.
pub(crate) async fn format_generated(path: &Path) -> Result<()> {
    match Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .arg("--config")
        .arg("skip_children=true")
        .arg(path)
        .output()
        .await
    {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(anyhow!(
            "生成したテストを整形できませんでした: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn repo_root() -> PathBuf {
    std::env::var_os("VIZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn push_line(tail: &Mutex<VecDeque<String>>, line: String) {
    let mut tail = tail.lock().expect("tail lock");
    if tail.len() == 60 {
        tail.pop_front();
    }
    tail.push_back(line);
}

async fn capture_lines<R>(reader: R, tail: &Mutex<VecDeque<String>>) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    while reader.read_until(b'\n', &mut bytes).await? != 0 {
        let line = String::from_utf8_lossy(&bytes)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        push_line(tail, line);
        bytes.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert!(crate::valid_name("demo_2"));
        assert!(!crate::valid_name("Demo"));
        assert!(!crate::valid_name("1demo"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn captures_both_streams_and_limits_tail() -> Result<()> {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 'out\\n'; printf 'err\\n' >&2"]);
        let (passed, code, tail) = execute(command, Duration::from_secs(5)).await?;
        assert!(passed);
        assert_eq!(code, Some(0));
        assert!(tail.lines().any(|line| line == "out"));
        assert!(tail.lines().any(|line| line == "err"));
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "i=1; while [ $i -le 65 ]; do printf '%s\\n' \"$i\"; i=$((i+1)); done",
        ]);
        let (_, _, tail) = execute(command, Duration::from_secs(5)).await?;
        assert_eq!(tail.lines().count(), 60);
        assert_eq!(tail.lines().next(), Some("6"));
        assert_eq!(tail.lines().last(), Some("65"));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn deadline_includes_inherited_pipes() -> Result<()> {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 0.2 &"]);
        let started = Instant::now();
        let (passed, code, tail) = execute(command, Duration::from_millis(20)).await?;
        assert!(!passed);
        assert_eq!(code, None);
        assert!(tail.contains("タイムアウト"));
        assert!(started.elapsed() < Duration::from_secs(1));
        // Let the deliberately short-lived descendant exit before the test ends.
        tokio::time::sleep(Duration::from_millis(220)).await;
        Ok(())
    }
}
