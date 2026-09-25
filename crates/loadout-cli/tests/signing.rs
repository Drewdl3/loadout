//! Signed-commit verification: a layer in
//! `policy.require_signed` only accepts commits signed by a company config signer.
//! Needs `ssh-keygen`; skipped (with a notice) where it isn't installed.

mod common;

use common::{Sandbox, skill};
use loadout_git::fixture::{FixtureRepo, ssh_keygen};

struct World {
    s: Sandbox,
    team: FixtureRepo,
    good: std::path::PathBuf,
    evil: std::path::PathBuf,
}

/// Skips outside CI; CI runners must be able to sign (so the test really
/// runs there).
fn skip(why: &str) -> Option<World> {
    assert!(std::env::var_os("CI").is_none(), "{why} in CI");
    eprintln!("{why}; skipping signing test");
    None
}

fn world() -> Option<World> {
    let s = Sandbox::with_claude();
    let keys = s.root.join("keys");
    let Some(good_pub) = ssh_keygen(&keys, "good") else {
        return skip("ssh-keygen not available");
    };
    ssh_keygen(&keys, "evil")?;
    let team = FixtureRepo::init(s.root.join("remotes/payments-skills"), s.git.clone());
    team.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: payments-skills\nlayer: team\ngroup: payments-dev\n---\n",
    )
    .write("skills/write-spec/SKILL.md", &skill("write-spec", "v1."));
    let good = keys.join("good");
    if team.commit_signed("v1", &good).is_none() {
        return skip("ssh commit signing not available");
    }
    let company_config = FixtureRepo::init(s.root.join("remotes/acme-config"), s.git.clone());
    company_config.write(
        "LOADOUT.md",
        &format!(
            "---\nloadout: 1\nname: acme-config\nlayer: company\ngroup: acme\ncompany:\n  name: acme\n  layers:\n    - {{ name: company, rank: 0 }}\n    - {{ name: team, rank: 20 }}\n  groups:\n    - layer: team\n      name: payments-dev\n      sources: [{:?}]\n      membership: {{ manual: true }}\n  signers:\n    - {{ principal: release@acme.example.com, ssh: {good_pub:?} }}\n  policy:\n    auto_apply: [company, team]\n    require_signed: [team]\n---\n",
            team.url()
        ),
    );
    company_config.commit("company_config");
    s.ok(&["init", &company_config.url(), "--non-interactive"]);
    s.ok(&["join", "team:payments-dev"]);
    Some(World {
        s,
        team,
        good,
        evil: keys.join("evil"),
    })
}

fn installed(w: &World) -> String {
    std::fs::read_to_string(w.s.skills_dir().join("write-spec/SKILL.md")).unwrap()
}

#[test]
fn tampered_signed_repo_is_refused() {
    let Some(w) = world() else { return };
    assert!(installed(&w).contains("v1."));
    let lock = std::fs::read_to_string(w.s.data.join("loadout.lock")).unwrap();

    // An unsigned commit (someone pushed around the signing process).
    w.team.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "evil unsigned."),
    );
    w.team.commit("unsigned");
    let out = w.s.run(&["sync"]);
    assert_eq!(out.code, 3, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("SIGNATURE CHECK FAILED"),
        "{}",
        out.stdout
    );
    assert!(installed(&w).contains("v1."));
    assert_eq!(
        std::fs::read_to_string(w.s.data.join("loadout.lock")).unwrap(),
        lock
    );

    // Signed, but by a key the company config doesn't list.
    w.team.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "evil signed."),
    );
    w.team.commit_signed("evil", &w.evil).unwrap();
    let out = w.s.run(&["sync", "--json"]);
    assert_eq!(out.code, 3);
    let r: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert!(
        r["signature_failures"][0]
            .as_str()
            .unwrap()
            .contains("REFUSED"),
        "{r}"
    );
    assert!(installed(&w).contains("v1."));

    // A properly signed commit applies.
    w.team.write(
        "skills/write-spec/SKILL.md",
        &skill("write-spec", "v2 signed."),
    );
    w.team.commit_signed("v2", &w.good).unwrap();
    let out = w.s.run(&["sync"]);
    assert_eq!(out.code, 0, "{}\n{}", out.stdout, out.stderr);
    assert!(installed(&w).contains("v2 signed."));
}
