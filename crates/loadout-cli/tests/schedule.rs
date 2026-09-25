//! `lo schedule`. The OS scheduler is never touched: the
//! test hook records the commands that would run.

mod common;

use common::Sandbox;

fn run(s: &Sandbox, args: &[&str], extra: &[(&str, &str)]) -> serde_json::Value {
    let log = s.scratch.join("scheduler.log");
    let mut env = vec![("LOADOUT_TEST_SCHEDULER", log.to_str().unwrap())];
    env.extend_from_slice(extra);
    let out = s.run_env(&[args, &["--json"]].concat(), &env);
    assert_eq!(out.code, 0, "{args:?}\n{}\n{}", out.stdout, out.stderr);
    serde_json::from_str(&out.stdout).unwrap()
}

fn log(s: &Sandbox) -> String {
    std::fs::read_to_string(s.scratch.join("scheduler.log")).unwrap_or_default()
}

#[test]
fn enable_status_disable_with_the_platform_scheduler() {
    let s = Sandbox::with_claude();
    let st = run(&s, &["schedule"], &[]);
    assert_eq!(st["enabled"], false);
    assert_eq!(st["interval"], "1h");

    let st = run(&s, &["schedule", "enable", "--interval", "30m"], &[]);
    assert_eq!(st["enabled"], true, "{st}");
    assert_eq!(st["interval"], "30m");
    let config = std::fs::read_to_string(s.config.join("config.toml")).unwrap();
    assert!(config.contains("sync_interval = \"30m\""), "{config}");

    let entry = std::path::PathBuf::from(st["entry"].as_str().unwrap());
    let backend = st["backend"].as_str().unwrap().to_owned();
    match backend.as_str() {
        "launchd" => {
            let plist = std::fs::read_to_string(&entry).unwrap();
            assert!(plist.contains("<integer>1800</integer>"), "{plist}");
            assert!(
                plist.contains("LOADOUT_DATA_DIR"),
                "sandbox env carried over"
            );
            assert!(
                log(&s).contains("launchctl bootstrap gui/501"),
                "{}",
                log(&s)
            );
        }
        "systemd" => {
            let timer = std::fs::read_to_string(&entry).unwrap();
            assert!(timer.contains("OnUnitActiveSec=1800s"), "{timer}");
            let service = std::fs::read_to_string(entry.with_extension("service")).unwrap();
            assert!(
                service.contains("Environment=\"LOADOUT_DATA_DIR="),
                "{service}"
            );
            assert!(
                log(&s).contains("systemctl --user enable --now loadout-sync.timer"),
                "{}",
                log(&s)
            );
        }
        "task-scheduler" => {
            let script = s.data.join("schedule-sync.cmd");
            let text = std::fs::read_to_string(&script).unwrap();
            assert!(text.contains("set \"LOADOUT_DATA_DIR="), "{text}");
            assert!(
                log(&s).contains("schtasks /Create /F /TN Loadout Sync /SC MINUTE /MO 30"),
                "{}",
                log(&s)
            );
        }
        other => panic!("unexpected backend {other}"),
    }

    let st = run(&s, &["schedule", "disable"], &[]);
    assert_eq!(st["enabled"], false, "{st}");
    if backend != "task-scheduler" {
        assert!(!entry.exists());
    }
}

#[cfg(unix)]
#[test]
fn cron_fallback_edits_only_its_own_line() {
    let s = Sandbox::with_claude();
    let crontab = s.scratch.join("scheduler.crontab");
    std::fs::create_dir_all(&s.scratch).unwrap();
    std::fs::write(&crontab, "MAILTO=me\n5 4 * * * backup\n").unwrap();
    let cron = [("LOADOUT_SCHEDULER", "cron")];
    let st = run(&s, &["schedule", "enable"], &cron);
    assert_eq!(st["backend"], "cron");
    assert_eq!(st["enabled"], true);
    let text = std::fs::read_to_string(&crontab).unwrap();
    assert!(text.starts_with("MAILTO=me\n5 4 * * * backup\n"), "{text}");
    assert!(text.contains("0 */1 * * * PATH="), "{text}");
    assert!(
        text.contains("'sync' '--quiet' '--non-interactive' '--exit-zero'"),
        "{text}"
    );
    run(&s, &["schedule", "enable"], &cron);
    let text = std::fs::read_to_string(&crontab).unwrap();
    assert_eq!(text.matches("# loadout-sync").count(), 1, "{text}");
    let st = run(&s, &["schedule", "disable"], &cron);
    assert_eq!(st["enabled"], false);
    assert_eq!(
        std::fs::read_to_string(&crontab).unwrap(),
        "MAILTO=me\n5 4 * * * backup\n"
    );
}

#[test]
fn exit_zero_always_succeeds() {
    let s = Sandbox::with_claude();
    let out = s.run(&["why", "skill/nothing", "--exit-zero"]);
    assert_eq!(out.code, 0);
    assert!(out.stderr.contains("error"), "{}", out.stderr);
}
