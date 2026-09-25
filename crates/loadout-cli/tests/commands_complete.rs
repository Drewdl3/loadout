//! Every documented command exists and parses.

mod common;

use common::Sandbox;

#[test]
fn every_documented_command_is_implemented() {
    let s = Sandbox::new();
    let help = s.ok(&["--help"]).stdout;
    let listed: Vec<&str> = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    for cmd in [
        "init",
        "profile",
        "join",
        "leave",
        "subscribe",
        "unsubscribe",
        "sync",
        "status",
        "diff",
        "approve",
        "list",
        "why",
        "enable",
        "disable",
        "prefer",
        "search",
        "info",
        "audit",
        "export",
        "import",
        "targets",
        "schedule",
        "secrets",
        "doctor",
        "new-source",
    ] {
        assert!(
            listed.contains(&cmd),
            "`lo {cmd}` missing from --help: {listed:?}"
        );
        let out = s.run(&[cmd, "--help"]);
        assert_eq!(out.code, 0, "lo {cmd} --help");
    }
    // Internal: hidden from --help but present.
    for hidden in ["mcp-run", "mcp-headers"] {
        assert!(!listed.contains(&hidden));
        assert_eq!(s.run(&[hidden, "--help"]).code, 0);
    }
    // Documented flags.
    let init = s.ok(&["init", "--help"]).stdout;
    assert!(init.contains("--non-interactive") && init.contains("--project"));
    let sync = s.ok(&["sync", "--help"]).stdout;
    for f in ["--dry-run", "--locked", "--if-stale", "--yes"] {
        assert!(sync.contains(f), "sync {f}");
    }
    let search = s.ok(&["search", "--help"]).stdout;
    for f in ["--kind", "--tag", "--layer", "--all-sources"] {
        assert!(search.contains(f), "search {f}");
    }
    let export = s.ok(&["export", "--help"]).stdout;
    assert!(export.contains("--qr") && export.contains("--latest"));
}

#[test]
fn tui_needs_a_terminal() {
    let s = Sandbox::new();
    let out = s.run(&["tui"]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("needs a terminal"), "{}", out.stderr);
}
