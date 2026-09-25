//! Distribution: `lo self-update` and `install.sh` against a
//! fake GitHub release on a loopback server.

mod common;

use std::collections::BTreeMap;

use common::Sandbox;
use loadout_cli::commands::self_update::target;
use loadout_members::mock_http::MockServer;
use sha2::{Digest, Sha256};

/// A fake v9.9.9 release whose binary prints `lo 9.9.9`.
fn release(s: &Sandbox, tamper: bool) -> MockServer {
    let target = target().unwrap();
    let name = format!("lo-9.9.9-{target}");
    let stage = s.root.join("release").join(&name);
    std::fs::create_dir_all(&stage).unwrap();
    let exe = if cfg!(windows) { "lo.exe" } else { "lo" };
    std::fs::write(stage.join(exe), "#!/bin/sh\necho \"lo 9.9.9\"\n").unwrap();
    let archive = format!("{name}.tar.gz");
    let status = std::process::Command::new("tar")
        .arg("-czf")
        .arg(s.root.join("release").join(&archive))
        .arg("-C")
        .arg(s.root.join("release"))
        .arg(&name)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(s.root.join("release").join(&archive)).unwrap();
    let mut hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if tamper {
        hash = "0".repeat(64);
    }
    let sums = format!("{hash}  {archive}\n");
    MockServer::start_with(move |base| {
        let json = format!(
            r#"{{"tag_name":"v9.9.9","assets":[{{"name":"{archive}","browser_download_url":"{base}/download/{archive}"}},{{"name":"SHA256SUMS","browser_download_url":"{base}/download/SHA256SUMS"}}]}}"#
        );
        let mut routes: BTreeMap<String, (u16, Vec<u8>)> = BTreeMap::new();
        routes.insert(format!("/download/{archive}"), (200, bytes));
        routes.insert("/download/SHA256SUMS".into(), (200, sums.into_bytes()));
        routes.insert("/releases/latest".into(), (200, json.into_bytes()));
        routes
    })
}

/// The release JSON must be asked for as JSON: GitHub answers 415 when it's
/// requested as `application/octet-stream`, which only suits the assets.
fn assert_release_json_accept(server: &MockServer) {
    let reqs = server.requests();
    let json = reqs
        .iter()
        .find(|r| r.path == "/releases/latest")
        .expect("release JSON requested");
    assert_eq!(json.accept.as_deref(), Some("application/vnd.github+json"));
}

#[test]
fn check_reports_a_newer_release() {
    let s = Sandbox::new();
    let server = release(&s, false);
    let out = s.run_env(
        &["self-update", "--check", "--json"],
        &[("LOADOUT_RELEASES_API", server.url())],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["latest"], "9.9.9");
    assert_eq!(v["update_available"], true);
    assert_eq!(v["updated"], false);
    assert_release_json_accept(&server);
}

#[cfg(unix)]
#[test]
fn self_update_verifies_and_replaces() {
    let s = Sandbox::new();
    let target_file = s.root.join("bin/loadout");
    std::fs::create_dir_all(target_file.parent().unwrap()).unwrap();
    std::fs::write(&target_file, "old").unwrap();

    let bad = release(&s, true);
    let out = s.run_env(
        &["self-update", "--path", target_file.to_str().unwrap()],
        &[("LOADOUT_RELEASES_API", bad.url())],
    );
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("checksum mismatch"), "{}", out.stderr);
    assert_eq!(std::fs::read_to_string(&target_file).unwrap(), "old");
    drop(bad);

    let good = release(&s, false);
    let out = s.run_env(
        &[
            "self-update",
            "--path",
            target_file.to_str().unwrap(),
            "--json",
        ],
        &[("LOADOUT_RELEASES_API", good.url())],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["updated"], true);
    assert_release_json_accept(&good);
    let new = std::process::Command::new(&target_file)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&new.stdout).trim(), "lo 9.9.9");
}

#[cfg(unix)]
#[test]
fn install_script_installs_a_verified_release() {
    if std::process::Command::new("curl")
        .arg("--version")
        .output()
        .is_err()
    {
        assert!(std::env::var_os("CI").is_none(), "curl missing in CI");
        return;
    }
    let s = Sandbox::new();
    let server = release(&s, false);
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
    let dir = s.root.join("installed");
    let out = std::process::Command::new("sh")
        .arg(&script)
        .arg("--dir")
        .arg(&dir)
        .env("LOADOUT_RELEASES_API", server.url())
        .env("HOME", &s.home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Installed lo 9.9.9"), "{stdout}");
    assert!(stdout.contains("Next: lo init"), "{stdout}");
    assert!(dir.join("lo").is_file());
    assert_release_json_accept(&server);
}
