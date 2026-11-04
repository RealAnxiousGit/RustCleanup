//! Running Linux command-line tools and reading what they print.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Output of a finished tool.
#[allow(dead_code)]
pub struct CmdOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[allow(dead_code)]
impl CmdOutput {
    /// The most useful single line of output, for log messages.
    pub fn summary(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        match (line.is_empty(), self.code) {
            (true, Some(c)) => format!("exit code {c}"),
            (true, None) => "terminated".to_string(),
            (false, _) => line.to_string(),
        }
    }
}

/// Finds a program on `PATH`, then in the sbin folders that `PATH` often
/// lacks for normal users. Never looks in the current folder.
pub fn which(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return is_executable(&p).then_some(p);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(
            [
                "/usr/local/sbin",
                "/usr/local/bin",
                "/usr/sbin",
                "/usr/bin",
                "/sbin",
                "/bin",
            ]
            .into_iter()
            .map(PathBuf::from),
        )
        .filter(|d| d.is_absolute())
        .map(|d| d.join(program))
        .find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// A `Command` for a system tool, found by full path when possible, with
/// English output so it can be parsed.
pub fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(which(program).unwrap_or_else(|| program.into()));
    cmd.stdin(Stdio::null()).env("LC_ALL", "C");
    cmd
}

/// Runs a tool and waits for it to finish.
///
/// `Err` means the process could not be started at all; a non-zero exit is
/// reported through `CmdOutput::success`.
pub fn run_hidden(program: &str, args: &[&str]) -> Result<CmdOutput, String> {
    let out = hidden_command(program)
        .args(args)
        .output()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    Ok(CmdOutput {
        success: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

