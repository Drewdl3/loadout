//! OS scheduler entries for periodic `lo sync`: a launchd
//! agent (macOS), a systemd user timer or a crontab line (Linux), a Task
//! Scheduler task (Windows). No resident daemon.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// launchd label, systemd unit stem, Task Scheduler task name.
pub const LABEL: &str = "com.loadout.sync";
pub const UNIT: &str = "loadout-sync";
pub const TASK: &str = "Loadout Sync";
/// Marks Loadout's crontab line.
pub const CRON_MARKER: &str = "# loadout-sync";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Launchd,
    Systemd,
    Cron,
    TaskScheduler,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Launchd => "launchd",
            Backend::Systemd => "systemd",
            Backend::Cron => "cron",
            Backend::TaskScheduler => "task-scheduler",
        }
    }
}

/// What the scheduled job runs.
#[derive(Debug, Clone)]
pub struct Job {
    /// Absolute path of `lo`.
    pub program: String,
    pub args: Vec<String>,
    /// Environment to set (PATH, LOADOUT_* overrides).
    pub env: Vec<(String, String)>,
    pub interval: Duration,
    /// Where output goes.
    pub log: PathBuf,
}

impl Job {
    pub fn new(
        program: String,
        interval: Duration,
        log: PathBuf,
        env: Vec<(String, String)>,
    ) -> Self {
        Job {
            program,
            args: ["sync", "--quiet", "--non-interactive", "--exit-zero"]
                .map(String::from)
                .to_vec(),
            env,
            interval,
            log,
        }
    }
}

/// Runs scheduler commands (`launchctl`, `systemctl`, `crontab`,
/// `schtasks`). Returns stdout; a non-zero exit is an error.
pub trait Runner {
    fn run(&self, program: &str, args: &[String], stdin: Option<&str>) -> Result<String>;
}

pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[String], stdin: Option<&str>) -> Result<String> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().with_context(|| format!("running {program}"))?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(input.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "`{program} {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Records commands in a file instead of running them (tests only, via
/// `LOADOUT_TEST_SCHEDULER`). `crontab -l` reads and `crontab -`
/// writes `<log>.crontab`.
pub struct RecordingRunner(pub PathBuf);

impl Runner for RecordingRunner {
    fn run(&self, program: &str, args: &[String], stdin: Option<&str>) -> Result<String> {
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.0)?;
        writeln!(log, "{program} {}", args.join(" "))?;
        let crontab = self.0.with_extension("crontab");
        match (program, args.first().map(String::as_str)) {
            ("crontab", Some("-l")) => Ok(std::fs::read_to_string(&crontab).unwrap_or_default()),
            ("crontab", Some("-")) => {
                std::fs::write(&crontab, stdin.unwrap_or_default())?;
                Ok(String::new())
            }
            ("id", _) => Ok("501\n".into()),
            _ => Ok(String::new()),
        }
    }
}

/// launchd agent plist.
pub fn launchd_plist(job: &Job) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut args = format!("    <string>{}</string>\n", esc(&job.program));
    for a in &job.args {
        args.push_str(&format!("    <string>{}</string>\n", esc(a)));
    }
    let mut env = String::new();
    for (k, v) in &job.env {
        env.push_str(&format!(
            "    <key>{}</key>\n    <string>{}</string>\n",
            esc(k),
            esc(v)
        ));
    }
    let log = esc(&job.log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>EnvironmentVariables</key>
  <dict>
{env}  </dict>
  <key>StartInterval</key>
  <integer>{}</integer>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        job.interval.as_secs()
    )
}

fn systemd_quote(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

/// systemd user `.service` and `.timer` units.
pub fn systemd_units(job: &Job) -> (String, String) {
    let mut exec = systemd_quote(&job.program);
    for a in &job.args {
        exec.push(' ');
        exec.push_str(&systemd_quote(a));
    }
    let env: String = job
        .env
        .iter()
        .map(|(k, v)| format!("Environment={}\n", systemd_quote(&format!("{k}={v}"))))
        .collect();
    let service = format!(
        "# Written by `lo schedule enable`.\n[Unit]\nDescription=Loadout sync\n\n[Service]\nType=oneshot\n{env}ExecStart={exec}\n"
    );
    let timer = format!(
        "# Written by `lo schedule enable`.\n[Unit]\nDescription=Loadout sync every {secs}s\n\n[Timer]\nOnActiveSec=30s\nOnUnitActiveSec={secs}s\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        secs = job.interval.as_secs()
    );
    (service, timer)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The crontab line (with [`CRON_MARKER`]).
pub fn cron_line(job: &Job) -> String {
    let mins = (job.interval.as_secs() / 60).max(1);
    let when = if mins < 60 {
        format!("*/{mins} * * * *")
    } else if mins < 24 * 60 {
        format!("0 */{} * * *", (mins / 60).max(1))
    } else {
        format!("0 0 */{} * *", (mins / (24 * 60)).max(1))
    };
    let env: String = job
        .env
        .iter()
        .map(|(k, v)| format!("{k}={} ", sh_quote(v)))
        .collect();
    let args: Vec<String> = job.args.iter().map(|a| sh_quote(a)).collect();
    format!(
        "{when} {env}{} {} >> {} 2>&1 {CRON_MARKER}",
        sh_quote(&job.program),
        args.join(" "),
        sh_quote(&job.log.to_string_lossy())
    )
}

/// Replaces Loadout's line in a crontab (or removes it with `None`).
pub fn edit_crontab(current: &str, line: Option<&str>) -> String {
    let mut out: Vec<&str> = current
        .lines()
        .filter(|l| !l.trim_end().ends_with(CRON_MARKER))
        .collect();
    if let Some(l) = line {
        out.push(l);
    }
    let mut s = out.join("\n");
    if !s.is_empty() {
        s.push('\n');
    }
    s
}

/// The Windows wrapper script (Task Scheduler's command line is limited to
/// 261 characters and can't set environment variables).
pub fn windows_script(job: &Job) -> String {
    let mut s = String::from("@echo off\r\nrem Written by `lo schedule enable`.\r\n");
    for (k, v) in &job.env {
        s.push_str(&format!("set \"{k}={v}\"\r\n"));
    }
    s.push_str(&format!(
        "\"{}\" {} >> \"{}\" 2>&1\r\n",
        job.program,
        job.args.join(" "),
        job.log.display()
    ));
    s
}

/// `schtasks /Create` arguments for running `script` every interval.
pub fn schtasks_create(script: &Path, interval: Duration) -> Vec<String> {
    let mins = (interval.as_secs() / 60).clamp(1, u64::MAX);
    let (sc, mo) = if mins < 1440 {
        ("MINUTE", mins)
    } else {
        ("DAILY", (mins / 1440).clamp(1, 365))
    };
    [
        "/Create",
        "/F",
        "/TN",
        TASK,
        "/SC",
        sc,
        "/MO",
        &mo.to_string(),
        "/TR",
        &format!("\"{}\"", script.display()),
    ]
    .map(String::from)
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> Job {
        Job::new(
            "/opt/lo".into(),
            Duration::from_secs(3600),
            "/h/.local/share/loadout/logs/schedule.log".into(),
            vec![("PATH".into(), "/usr/bin:/bin".into())],
        )
    }

    #[test]
    fn renders_plist() {
        insta::assert_snapshot!(launchd_plist(&job()));
    }

    #[test]
    fn renders_systemd_units() {
        let (service, timer) = systemd_units(&job());
        insta::assert_snapshot!(format!("{service}\n----\n{timer}"));
    }

    #[test]
    fn cron_lines_and_edits() {
        let mut j = job();
        assert!(cron_line(&j).starts_with("0 */1 * * * PATH='/usr/bin:/bin' '/opt/lo' 'sync'"));
        j.interval = Duration::from_secs(15 * 60);
        assert!(cron_line(&j).starts_with("*/15 * * * * "));
        j.interval = Duration::from_secs(2 * 86_400);
        assert!(cron_line(&j).starts_with("0 0 */2 * * "));
        assert!(cron_line(&j).ends_with(CRON_MARKER));

        let mine = "MAILTO=me\n5 4 * * * backup\n";
        let with = edit_crontab(mine, Some(&cron_line(&j)));
        assert!(with.starts_with(mine) && with.contains(CRON_MARKER));
        let again = edit_crontab(&with, Some("* * * * * x # loadout-sync"));
        assert_eq!(again.matches(CRON_MARKER).count(), 1);
        assert_eq!(edit_crontab(&again, None), mine);
        assert_eq!(edit_crontab("", None), "");
    }

    #[test]
    fn windows_script_and_schtasks() {
        let s = windows_script(&job());
        assert!(s.contains("set \"PATH=/usr/bin:/bin\"\r\n"));
        assert!(s.contains("\"/opt/lo\" sync --quiet --non-interactive --exit-zero >> "));
        let a = schtasks_create(
            Path::new(r"C:\data\schedule-sync.cmd"),
            Duration::from_secs(3600),
        );
        assert_eq!(
            a[..8].join(" "),
            "/Create /F /TN Loadout Sync /SC MINUTE /MO 60"
        );
        let a = schtasks_create(Path::new("x"), Duration::from_secs(3 * 86_400));
        assert_eq!(&a[5..8], ["DAILY", "/MO", "3"]);
    }
}
