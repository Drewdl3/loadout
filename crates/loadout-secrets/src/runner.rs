//! Running helper tools (`op`, `vault`, `sops`).

use std::path::Path;
use std::process::{Command, Stdio};

/// A finished tool run. `stdout` may hold a secret: callers wrap it in a
/// `SecretString` immediately and never log it.
#[derive(Clone)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl std::fmt::Debug for RunOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunOutput")
            .field("success", &self.success)
            .field("stdout", &"[redacted]")
            .field("stderr", &self.stderr)
            .finish()
    }
}

pub trait Runner {
    /// Runs `program` with `args` (no shell), stdin closed. A missing
    /// program is an `io::ErrorKind::NotFound` error.
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<RunOutput>;
}

/// Runs real processes.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> std::io::Result<RunOutput> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        let out = cmd.output()?;
        Ok(RunOutput {
            success: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
