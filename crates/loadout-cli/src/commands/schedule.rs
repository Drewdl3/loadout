//! `lo schedule [enable|disable|status] [--interval]`.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use loadout_targets::fsutil::atomic_write;
use schemars::JsonSchema;
use serde::Serialize;

use crate::ctx::{Ctx, Report};
use crate::exit;
use crate::review::State;
use crate::schedule::{self, Backend, Job, RecordingRunner, Runner, SystemRunner};

#[derive(Debug, Args)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: Option<ScheduleCommand>,
}

#[derive(Debug, Subcommand)]
pub enum ScheduleCommand {
    /// Install an OS scheduler entry that runs `lo sync` periodically.
    Enable {
        /// How often (e.g. 30m, 1h, 1d); saved as `sync_interval`.
        #[arg(long)]
        interval: Option<String>,
    },
    /// Remove the scheduler entry.
    Disable,
    /// Show whether periodic sync is set up (the default).
    Status,
}

/// `--json` output of `lo schedule`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ScheduleReport {
    pub backend: Backend,
    pub enabled: bool,
    /// `sync_interval`.
    pub interval: String,
    /// The scheduler entry (a file path, or `crontab`).
    pub entry: String,
    pub log: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<String>,
}

impl Report for ScheduleReport {
    fn human(&self, out: &mut String) -> std::fmt::Result {
        writeln!(
            out,
            "Periodic sync: {} ({}, every {})",
            if self.enabled { "enabled" } else { "disabled" },
            self.backend.as_str(),
            self.interval
        )?;
        writeln!(out, "Entry:     {}", self.entry)?;
        writeln!(out, "Log:       {}", self.log)?;
        writeln!(
            out,
            "Last sync: {}",
            self.last_sync.as_deref().unwrap_or("never")
        )
    }
}

struct Setup {
    backend: Backend,
    runner: Box<dyn Runner>,
    home: PathBuf,
    config_home: PathBuf,
    data: PathBuf,
}

impl Setup {
    fn new(ctx: &Ctx) -> Result<Self> {
        let runner: Box<dyn Runner> = match loadout_targets::testhook::scheduler_log() {
            Some(log) => Box::new(RecordingRunner(log)),
            None => Box::new(SystemRunner),
        };
        let backend = pick_backend(runner.as_ref())?;
        let home = ctx.paths.home.clone();
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        Ok(Setup {
            backend,
            runner,
            home,
            config_home,
            data: ctx.paths.data_dir.clone(),
        })
    }

    fn plist(&self) -> PathBuf {
        self.home
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", schedule::LABEL))
    }

    fn unit(&self, ext: &str) -> PathBuf {
        self.config_home
            .join("systemd/user")
            .join(format!("{}.{ext}", schedule::UNIT))
    }

    fn script(&self) -> PathBuf {
        self.data.join("schedule-sync.cmd")
    }

    fn log(&self) -> PathBuf {
        self.data.join("logs").join("schedule.log")
    }

    fn run(&self, program: &str, args: &[&str], stdin: Option<&str>) -> Result<String> {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        self.runner.run(program, &args, stdin)
    }

    fn gui_domain(&self) -> Result<String> {
        Ok(format!("gui/{}", self.run("id", &["-u"], None)?.trim()))
    }

    fn entry(&self) -> String {
        match self.backend {
            Backend::Launchd => self.plist().display().to_string(),
            Backend::Systemd => self.unit("timer").display().to_string(),
            Backend::Cron => "crontab".into(),
            Backend::TaskScheduler => format!("Task Scheduler \"{}\"", schedule::TASK),
        }
    }

    fn enabled(&self) -> bool {
        match self.backend {
            Backend::Launchd => self.plist().is_file(),
            Backend::Systemd => self.unit("timer").is_file(),
            Backend::Cron => self
                .run("crontab", &["-l"], None)
                .is_ok_and(|t| t.contains(schedule::CRON_MARKER)),
            Backend::TaskScheduler => self.script().is_file(),
        }
    }

    fn enable(&self, job: &Job) -> Result<()> {
        match self.backend {
            Backend::Launchd => {
                let plist = self.plist();
                atomic_write(&plist, schedule::launchd_plist(job).as_bytes(), false)?;
                let domain = self.gui_domain()?;
                let _ = self.run(
                    "launchctl",
                    &["bootout", &format!("{domain}/{}", schedule::LABEL)],
                    None,
                );
                self.run(
                    "launchctl",
                    &["bootstrap", &domain, &plist.to_string_lossy()],
                    None,
                )?;
            }
            Backend::Systemd => {
                let (service, timer) = schedule::systemd_units(job);
                atomic_write(&self.unit("service"), service.as_bytes(), false)?;
                atomic_write(&self.unit("timer"), timer.as_bytes(), false)?;
                self.run("systemctl", &["--user", "daemon-reload"], None)?;
                self.run(
                    "systemctl",
                    &[
                        "--user",
                        "enable",
                        "--now",
                        &format!("{}.timer", schedule::UNIT),
                    ],
                    None,
                )?;
                // Pick up a changed interval on an already-running timer.
                self.run(
                    "systemctl",
                    &["--user", "restart", &format!("{}.timer", schedule::UNIT)],
                    None,
                )?;
            }
            Backend::Cron => {
                let current = self.run("crontab", &["-l"], None).unwrap_or_default();
                let line = schedule::cron_line(job);
                self.run(
                    "crontab",
                    &["-"],
                    Some(&schedule::edit_crontab(&current, Some(&line))),
                )?;
            }
            Backend::TaskScheduler => {
                let script = self.script();
                atomic_write(&script, schedule::windows_script(job).as_bytes(), false)?;
                let args = schedule::schtasks_create(&script, job.interval);
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                self.run("schtasks", &args, None)?;
            }
        }
        Ok(())
    }

    fn disable(&self) -> Result<()> {
        match self.backend {
            Backend::Launchd => {
                let domain = self.gui_domain()?;
                let _ = self.run(
                    "launchctl",
                    &["bootout", &format!("{domain}/{}", schedule::LABEL)],
                    None,
                );
                remove(&self.plist())?;
            }
            Backend::Systemd => {
                let _ = self.run(
                    "systemctl",
                    &[
                        "--user",
                        "disable",
                        "--now",
                        &format!("{}.timer", schedule::UNIT),
                    ],
                    None,
                );
                remove(&self.unit("timer"))?;
                remove(&self.unit("service"))?;
                let _ = self.run("systemctl", &["--user", "daemon-reload"], None);
            }
            Backend::Cron => {
                let current = self.run("crontab", &["-l"], None).unwrap_or_default();
                if current.contains(schedule::CRON_MARKER) {
                    self.run(
                        "crontab",
                        &["-"],
                        Some(&schedule::edit_crontab(&current, None)),
                    )?;
                }
            }
            Backend::TaskScheduler => {
                let _ = self.run("schtasks", &["/Delete", "/F", "/TN", schedule::TASK], None);
                remove(&self.script())?;
            }
        }
        Ok(())
    }
}

fn remove(p: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", p.display())),
    }
}

/// The platform's scheduler; `LOADOUT_SCHEDULER` overrides (`cron` or
/// `systemd` on Unix).
fn pick_backend(runner: &dyn Runner) -> Result<Backend> {
    if let Ok(choice) = std::env::var("LOADOUT_SCHEDULER") {
        return match (choice.as_str(), cfg!(windows), cfg!(target_os = "macos")) {
            ("cron", false, _) => Ok(Backend::Cron),
            ("systemd", false, false) => Ok(Backend::Systemd),
            ("launchd", false, true) => Ok(Backend::Launchd),
            ("task-scheduler", true, _) => Ok(Backend::TaskScheduler),
            _ => bail!("LOADOUT_SCHEDULER={choice} is not available on this platform"),
        };
    }
    Ok(if cfg!(windows) {
        Backend::TaskScheduler
    } else if cfg!(target_os = "macos") {
        Backend::Launchd
    } else if runner
        .run(
            "systemctl",
            &["--user".to_owned(), "show-environment".to_owned()],
            None,
        )
        .is_ok()
    {
        Backend::Systemd
    } else {
        Backend::Cron
    })
}

pub fn run(ctx: &Ctx, args: ScheduleArgs) -> Result<u8> {
    let setup = Setup::new(ctx)?;
    let mut doc = ctx.load_config_doc()?;
    match args.command.unwrap_or(ScheduleCommand::Status) {
        ScheduleCommand::Enable { interval } => {
            if let Some(i) = &interval {
                loadout_model::duration::parse_duration(i)?;
                doc.set_sync_interval(i);
                ctx.save_config(&doc)?;
            }
            let config = doc.config();
            let every = loadout_model::duration::parse_duration(config.sync_interval())?;
            std::fs::create_dir_all(setup.log().parent().expect("log dir"))?;
            let mut env: Vec<(String, String)> = Vec::new();
            if let Some(path) = std::env::var_os("PATH") {
                env.push(("PATH".into(), path.to_string_lossy().into_owned()));
            }
            for k in ["LOADOUT_HOME", "LOADOUT_CONFIG_DIR", "LOADOUT_DATA_DIR"] {
                if let Some(v) = std::env::var_os(k) {
                    env.push((k.into(), v.to_string_lossy().into_owned()));
                }
            }
            let job = Job::new(crate::engine::loadout_bin(), every, setup.log(), env);
            setup.enable(&job)?;
        }
        ScheduleCommand::Disable => setup.disable()?,
        ScheduleCommand::Status => {}
    }
    let config = doc.config();
    let state = State::load(&ctx.paths.state_file())?;
    ctx.emit(&ScheduleReport {
        backend: setup.backend,
        enabled: setup.enabled(),
        interval: config.sync_interval().to_owned(),
        entry: setup.entry(),
        log: setup.log().display().to_string(),
        last_sync: state.last_sync,
    })?;
    Ok(exit::OK)
}
