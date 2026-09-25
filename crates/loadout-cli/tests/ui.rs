//! In `lo ui`, toggle and prefer items and
//! see them applied; edit a source item and export it — a pull request is
//! opened against a fixture repo (mock GitHub API), and the repo's default
//! branch is never pushed.

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Stdio};

use common::{Sandbox, skill};
use loadout_git::fixture::FixtureRepo;
use loadout_members::mock_http::MockServer;
use serde_json::{Value, json};

const GH_URL: &str = "https://github.com/acme/app-skills";

struct Ui {
    child: Child,
    base: String,
    token: String,
    port: u16,
}

impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Ui {
    fn start(s: &Sandbox, api: &str) -> Ui {
        Ui::start_env(s, api, &[])
    }

    fn start_env(s: &Sandbox, api: &str, env: &[(&str, &str)]) -> Ui {
        let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("lo"));
        cmd.env_clear();
        for (k, v) in s.cmd().get_envs() {
            if let Some(v) = v {
                cmd.env(k, v);
            }
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.env("LOADOUT_GITHUB_API", api)
            .env("GH_TOKEN", "gh-test-token")
            .args(["ui", "--no-open", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line}: {e}"));
        let url = v["url"].as_str().unwrap().to_owned();
        let (base, token) = url.split_once("/?token=").unwrap();
        Ui {
            child,
            base: base.to_owned(),
            token: token.to_owned(),
            port: v["port"].as_u64().unwrap() as u16,
        }
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into()
    }

    fn get(&self, path: &str) -> (u16, Value) {
        let mut r = Self::agent()
            .get(format!("{}{path}", self.base))
            .header("X-Loadout-Token", &self.token)
            .call()
            .unwrap();
        let status = r.status().as_u16();
        (
            status,
            serde_json::from_str(&r.body_mut().read_to_string().unwrap()).unwrap(),
        )
    }

    fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let mut r = Self::agent()
            .post(format!("{}{path}", self.base))
            .header("X-Loadout-Token", &self.token)
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .unwrap();
        let status = r.status().as_u16();
        (
            status,
            serde_json::from_str(&r.body_mut().read_to_string().unwrap()).unwrap(),
        )
    }

    fn run(&self, args: &[&str]) -> Value {
        let (status, v) = self.post("/api/run", json!({ "args": args }));
        assert_eq!(status, 200, "{args:?}: {v}");
        v
    }
}

fn source(
    s: &Sandbox,
    name: &str,
    layer: &str,
    group: &str,
    files: &[(&str, &str)],
) -> FixtureRepo {
    let r = FixtureRepo::init(s.root.join("remotes").join(name), s.git.clone());
    r.write(
        "LOADOUT.md",
        &format!("---\nloadout: 1\nname: {name}\nlayer: {layer}\ngroup: {group}\n---\n"),
    );
    for (p, c) in files {
        r.write(p, c);
    }
    r.commit("init");
    r
}

fn world() -> (Sandbox, FixtureRepo) {
    let s = Sandbox::with_claude();
    let app = source(
        &s,
        "app-skills",
        "team",
        "payments-dev",
        &[(
            "skills/write-spec/SKILL.md",
            &skill("write-spec", "App version."),
        )],
    );
    source(
        &s,
        "other-skills",
        "team",
        "payments-dev",
        &[
            (
                "skills/write-spec/SKILL.md",
                &skill("write-spec", "Other version."),
            ),
            (
                "skills/extra/SKILL.md",
                "---\ndescription: Opt-in extra.\nloadout: { mode: default-off }\n---\nExtra.\n",
            ),
        ],
    );
    source(
        &s,
        "org-skills",
        "org",
        "eng",
        &[(
            "skills/standards/SKILL.md",
            "---\ndescription: Org standards.\nloadout: { locked: true }\n---\nStandards.\n",
        )],
    );
    // The app repo "lives on GitHub": git maps the https URL to the fixture.
    let file_url = format!(
        "file:///{}",
        app.path
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    std::fs::write(
        s.scratch.join("gitconfig"),
        format!(
            "[user]\n\tname = Ada Lovelace\n\temail = ada@example.com\n[url \"{file_url}\"]\n\tinsteadOf = {GH_URL}\n"
        ),
    )
    .unwrap();
    s.ok(&["subscribe", GH_URL]);
    s.ok(&[
        "subscribe",
        &s.root.join("remotes/other-skills").to_string_lossy(),
    ]);
    s.ok(&[
        "subscribe",
        &s.root.join("remotes/org-skills").to_string_lossy(),
    ]);
    let out = s.run(&["sync"]);
    assert_eq!(out.code, 4, "equal-rank conflict expected\n{}", out.stderr);
    (s, app)
}

fn installed(s: &Sandbox, name: &str) -> Option<String> {
    std::fs::read_to_string(s.skills_dir().join(name).join("SKILL.md")).ok()
}

#[test]
fn ui_toggles_prefers_and_exports_a_pull_request() {
    let (s, app) = world();
    let mut routes = BTreeMap::new();
    routes.insert(
        "/repos/acme/app-skills/pulls".to_owned(),
        (
            201,
            r#"{"html_url":"https://github.com/acme/app-skills/pull/7","number":7}"#.to_owned(),
        ),
    );
    let github = MockServer::start(routes);
    let ui = Ui::start(&s, github.url());

    // Access control.
    let agent = Ui::agent();
    let r = agent.get(format!("{}/", ui.base)).call().unwrap();
    assert_eq!(r.status().as_u16(), 403, "no token, no page");
    let mut r = agent
        .get(format!("{}/?token={}", ui.base, ui.token))
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert!(
        r.body_mut()
            .read_to_string()
            .unwrap()
            .contains("<title>Loadout</title>")
    );
    let r = agent.get(format!("{}/api/list", ui.base)).call().unwrap();
    assert_eq!(r.status().as_u16(), 401, "API needs the token header");
    // DNS rebinding: a foreign Host header is refused.
    let mut raw = std::net::TcpStream::connect(("127.0.0.1", ui.port)).unwrap();
    write!(
        raw,
        "GET /api/list HTTP/1.1\r\nHost: evil.example.com\r\nX-Loadout-Token: {}\r\nConnection: close\r\n\r\n",
        ui.token
    )
    .unwrap();
    let mut resp = String::new();
    raw.read_to_string(&mut resp).unwrap();
    assert!(resp.starts_with("HTTP/1.1 403"), "{resp}");
    let (status, v) = ui.post("/api/run", json!({"args": ["secrets", "set", "x"]}));
    assert_eq!(status, 403, "{v}");

    // Toggle: enable a default-off item and see it installed.
    assert_eq!(installed(&s, "extra"), None);
    ui.run(&["enable", "other-skills:skill/extra"]);
    assert!(installed(&s, "extra").unwrap().contains("Extra."));

    // Prefer: settle the equal-rank conflict.
    let (_, why) = ui.get("/api/why?key=skill%2Fwrite-spec");
    assert_eq!(
        why["output"]["winner"], "app-skills:skill/write-spec",
        "lexicographic pick"
    );
    ui.run(&["prefer", "other-skills:skill/write-spec"]);
    assert!(
        installed(&s, "write-spec")
            .unwrap()
            .contains("Other version.")
    );
    let (_, list) = ui.get("/api/list");
    assert!(
        list["output"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == "other-skills:skill/write-spec")
    );

    // Items locked by a more general layer can't be edited from a team source.
    let (status, v) = ui.post(
        "/api/source-item",
        json!({"source": "app-skills", "key": "skill/standards", "text": "---\ndescription: mine\n---\n"}),
    );
    assert_eq!(status, 400);
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("locked by org-skills:skill/standards"),
        "{v}"
    );

    // Edit a source item and export it as a pull request.
    let main_before = s.git.head(&app.path).unwrap();
    let (_, item) = ui.get("/api/source-item?source=app-skills&key=skill%2Fwrite-spec");
    assert!(
        item["output"]["text"]
            .as_str()
            .unwrap()
            .contains("App version.")
    );
    let edited = skill("write-spec", "App version, improved.");
    let (status, v) = ui.post(
        "/api/source-item",
        json!({"source": "app-skills", "key": "skill/write-spec", "text": edited}),
    );
    assert_eq!(status, 200, "{v}");
    let (status, v) = ui.post(
        "/api/export",
        json!({"source": "app-skills", "message": "Improve the spec skill"}),
    );
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["code"], 0, "{v}");
    let out = &v["output"];
    assert_eq!(out["pull_request"]["number"], 7, "{v}");
    let branch = out["branch"].as_str().unwrap().to_owned();
    assert!(
        branch.starts_with("loadout/ada-lovelace/improve-the-spec-skill-"),
        "{branch}"
    );

    // The PR request: head = the new branch, base = main.
    let reqs = github.requests();
    let pr = reqs
        .iter()
        .find(|r| r.path == "/repos/acme/app-skills/pulls")
        .expect("PR requested");
    assert_eq!(pr.method, "POST");
    assert_eq!(pr.authorization.as_deref(), Some("Bearer gh-test-token"));
    let body: Value = serde_json::from_str(&pr.body).unwrap();
    assert_eq!(body["head"], branch.as_str());
    assert_eq!(body["base"], "main");
    assert_eq!(body["title"], "Improve the spec skill");

    // The fixture repo got the branch; main is untouched.
    assert_eq!(s.git.head(&app.path).unwrap(), main_before);
    let on_branch = s
        .git
        .run(
            Some(&app.path),
            ["show", &format!("{branch}:skills/write-spec/SKILL.md")],
        )
        .unwrap();
    assert!(on_branch.contains("App version, improved."));
    let on_main = s
        .git
        .run(Some(&app.path), ["show", "main:skills/write-spec/SKILL.md"])
        .unwrap();
    assert!(on_main.contains("App version.") && !on_main.contains("improved"));

    // Nothing left to export; the default branch can't be targeted.
    let (_, v) = ui.post("/api/export", json!({"source": "app-skills"}));
    assert!(
        v["output"]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no edits"),
        "{v}"
    );
    drop(ui);
    let w = s.json(&["export-source", "app-skills", "--prepare"]);
    let path = std::path::PathBuf::from(w["path"].as_str().unwrap());
    std::fs::write(
        path.join("skills/write-spec/SKILL.md"),
        "---\ndescription: x\n---\n",
    )
    .unwrap();
    let out = s.run(&["export-source", "app-skills", "--branch", "main"]);
    assert_ne!(out.code, 0);
    assert!(
        out.stderr.contains("must start with `loadout/`"),
        "{}",
        out.stderr
    );
    assert_eq!(s.git.head(&app.path).unwrap(), main_before);
}

#[test]
fn move_item_between_sources() {
    let (s, _app) = world();
    let github = MockServer::start(BTreeMap::new());
    let ui = Ui::start(&s, github.url());
    let (status, v) = ui.post(
        "/api/move-item",
        json!({"key": "skill/extra", "from": "other-skills", "to": "app-skills"}),
    );
    assert_eq!(status, 200, "{v}");
    let (_, a) = ui.get("/api/source-item?source=app-skills&key=skill%2Fextra");
    assert!(
        a["output"]["text"]
            .as_str()
            .unwrap()
            .contains("Opt-in extra.")
    );
    let (_, b) = ui.get("/api/source-item?source=other-skills&key=skill%2Fextra");
    assert_eq!(b["output"]["text"], "");
    let (status, _) = ui.post(
        "/api/move-item",
        json!({"key": "skill/extra", "from": "other-skills", "to": "app-skills"}),
    );
    assert_eq!(status, 400, "already moved");
}

/// Upstream sources, preview-then-subscribe, search facets and templates
/// through the UI's API.
#[test]
fn preview_subscribe_and_use_a_template() {
    let s = Sandbox::with_claude();
    let eng = FixtureRepo::init(s.root.join("remotes/eng-skills"), s.git.clone());
    eng.write(
        "LOADOUT.md",
        "---\nloadout: 1\nname: eng-skills\nlayer: org\ngroup: eng\n---\n",
    )
    .write(
        "skills/style/SKILL.md",
        "---\ndescription: Style.\nloadout: { tags: [style], applies_to: { role: [developer] } }\n---\n",
    )
    .write(
        "templates/pr-workflow/SKILL.md",
        "---\nname: pr-workflow\ndescription: PRs for {{team}}.\ntemplate:\n  variables: [{ name: team }]\n---\n# {{team}}\n",
    );
    eng.commit("eng");
    let squad = source(
        &s,
        "checkout-squad",
        "squad",
        "checkout",
        &[
            ("skills/deploy/SKILL.md", &skill("deploy", "Deploy.")),
            (
                "LOADOUT.md",
                "---\nloadout: 1\nname: checkout-squad\nlayer: squad\ngroup: checkout\nupstream: [../eng-skills]\n---\n",
            ),
        ],
    );
    let ui = Ui::start(&s, "http://127.0.0.1:9");

    // Preview: the squad and what it builds on; nothing changes.
    let url = squad.url();
    let (status, p) = ui.get(&format!(
        "/api/preview?url={}",
        url.replace('%', "%25").replace(' ', "%20")
    ));
    assert_eq!(status, 200, "{p}");
    let sources = p["output"]["sources"].as_array().unwrap();
    assert_eq!(sources[0]["name"], "checkout-squad");
    assert_eq!(sources[1]["name"], "eng-skills");
    assert_eq!(sources[1]["via"], "checkout-squad");
    assert_eq!(
        sources[1]["templates"][0],
        "eng-skills:template/pr-workflow"
    );
    assert!(!s.config.join("config.toml").exists());
    let (status, _) = ui.get("/api/preview?url=--help");
    assert_eq!(status, 400);

    ui.run(&["subscribe", &url]);
    ui.run(&["sync"]);
    let (_, src) = ui.get("/api/sources");
    let eng_entry = src["output"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "eng-skills")
        .unwrap()
        .clone();
    assert_eq!(eng_entry["via"], "checkout-squad");

    // Search returns everything, with the fields the filters use.
    let (_, hits) = ui.get("/api/search?q=");
    let hits = hits["output"]["hits"].as_array().unwrap().clone();
    let style = hits.iter().find(|h| h["name"] == "style").unwrap();
    assert_eq!(style["roles"], json!(["developer"]));
    assert_eq!(style["tags"], json!(["style"]));
    assert_eq!(style["source"], "eng-skills");

    // Templates.
    let (_, t) = ui.get("/api/templates");
    assert_eq!(
        t["output"]["templates"][0]["id"],
        "eng-skills:template/pr-workflow"
    );
    let (status, used) = ui.post(
        "/api/template-use",
        json!({"template": "eng-skills:template/pr-workflow", "into": "checkout-squad",
               "name": "checkout-prs", "values": {"team": "Checkout"}}),
    );
    assert_eq!(status, 200, "{used}");
    assert_eq!(used["code"], 0, "{used}");
    assert_eq!(used["output"]["id"], "checkout-squad:skill/checkout-prs");
    let written = std::path::PathBuf::from(used["output"]["path"].as_str().unwrap());
    assert!(
        std::fs::read_to_string(written.join("SKILL.md"))
            .unwrap()
            .contains("# Checkout")
    );
    for bad in [
        json!({"template": "-x", "into": "checkout-squad"}),
        json!({"template": "pr-workflow", "into": "checkout-squad", "values": {"--x": "1"}}),
        json!({"template": "pr-workflow"}),
    ] {
        let (status, _) = ui.post("/api/template-use", bad.clone());
        assert_eq!(status, 400, "{bad}");
    }

    // Make a template in a working clone: checked, then written where
    // `lo template` looks for it.
    let text = "---\nname: oncall\ndescription: On-call for {{team}}.\ntemplate:\n  description: An on-call skill.\n  variables:\n    - { name: team }\n    - { name: pager, default: \"#oncall\" }\n---\n# {{team}} on-call\n\nPage {{ pager }}.\n";
    let (_, empty) = ui.get("/api/source-template?source=checkout-squad&name=oncall");
    assert_eq!(empty["output"]["text"], "", "{empty}");
    let new = json!({"source": "checkout-squad", "name": "oncall", "text": text, "create": true});
    let (status, made) = ui.post("/api/source-template", new.clone());
    assert_eq!(status, 200, "{made}");
    assert_eq!(made["output"]["path"], "templates/oncall/SKILL.md");
    let (_, back) = ui.get("/api/source-template?source=checkout-squad&name=oncall");
    assert_eq!(back["output"]["text"], text);
    let (status, dup) = ui.post("/api/source-template", new);
    assert_eq!(status, 400);
    assert!(
        dup["error"].as_str().unwrap().contains("already has"),
        "{dup}"
    );
    let (status, undeclared) = ui.post(
        "/api/source-template",
        json!({"source": "checkout-squad", "name": "other", "text": text.replace("{{ pager }}", "{{channel}}")}),
    );
    assert_eq!(status, 400);
    assert!(
        undeclared["error"]
            .as_str()
            .unwrap()
            .contains("{{channel}}"),
        "{undeclared}"
    );
}

#[test]
fn adopt_installed_skills_into_a_source() {
    let (s, _) = world();
    let mine = s.skills_dir().join("my-debugging");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("SKILL.md"), skill("my-debugging", "Mine.")).unwrap();
    let ui = Ui::start(&s, "http://127.0.0.1:9");

    let (status, found) = ui.get("/api/adopt");
    assert_eq!(status, 200, "{found}");
    let names: Vec<&str> = found["output"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["my-debugging"], "{found}");

    let (status, r) = ui.post(
        "/api/adopt",
        json!({"into": "other-skills", "skills": ["my-debugging"]}),
    );
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["code"], 0, "{r}");
    assert_eq!(
        r["output"]["adopted"][0]["id"],
        "other-skills:skill/my-debugging"
    );
    // The new skill is readable through the editor endpoint.
    let (_, item) = ui.get("/api/source-item?source=other-skills&key=skill/my-debugging");
    assert!(item["output"]["text"].as_str().unwrap().contains("Mine."));

    for bad in [
        json!({"skills": ["my-debugging"]}),
        json!({"into": "other-skills"}),
        json!({"into": "other-skills", "skills": ["--all"]}),
        json!({"into": "-x", "skills": ["a"]}),
        json!({"into": "other-skills", "skills": ["a"], "path": "--dir"}),
    ] {
        let (status, _) = ui.post("/api/adopt", bad.clone());
        assert_eq!(status, 400, "{bad}");
    }
    let (status, _) = ui.get("/api/adopt?path=--all");
    assert_eq!(status, 400);
}

#[test]
fn layers_lists_layers_in_rank_order_with_each_winner() {
    let (s, _app) = world();
    let ui = Ui::start(&s, "http://127.0.0.1:9");
    let (status, v) = ui.get("/api/layers");
    assert_eq!(status, 200, "{v}");
    let layers: Vec<&str> = v["output"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        layers.first(),
        Some(&"company"),
        "broadest first: {layers:?}"
    );
    assert_eq!(layers.last(), Some(&"user"), "you last: {layers:?}");
    let pos = |n: &str| layers.iter().position(|s| *s == n).unwrap();
    assert!(pos("org") < pos("team"));

    let res = &v["output"]["resolution"];
    assert_eq!(
        res["skill/write-spec"]["winner"],
        "app-skills:skill/write-spec"
    );
    assert_eq!(res["skill/write-spec"]["rule"], "conflict_tie_break");
    assert_eq!(
        res["skill/standards"]["winner"],
        "org-skills:skill/standards"
    );
    ui.run(&["prefer", "other-skills:skill/write-spec"]);
    let (_, v) = ui.get("/api/layers");
    assert_eq!(
        v["output"]["resolution"]["skill/write-spec"]["winner"],
        "other-skills:skill/write-spec"
    );
    assert_eq!(
        v["output"]["resolution"]["skill/write-spec"]["rule"],
        "preferred"
    );
}

/// Promote an item to a broader layer, clone one down to a personal source
/// (adding you as a co-author), edit its fields, and find items by author.
#[test]
fn promote_clone_down_and_edit_with_authors() {
    let (s, _app) = world();
    source(
        &s,
        "ada-skills",
        "user",
        "ada-lovelace",
        &[(
            "skills/mine/SKILL.md",
            "---\ndescription: Mine.\nloadout: { author: Ada Lovelace, coAuthors: [Grace Hopper] }\n---\nMine.\n",
        )],
    );
    s.ok(&[
        "subscribe",
        &s.root.join("remotes/ada-skills").to_string_lossy(),
    ]);
    assert_eq!(s.run(&["sync"]).code, 4);

    // Authors are searchable and shown by `info`.
    let r = s.json(&["search", "", "--author", "grace", "--limit", "0"]);
    let ids: Vec<&str> = r["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["ada-skills:skill/mine"]);
    assert_eq!(r["hits"][0]["co_authors"], json!(["Grace Hopper"]));
    let info = s.ok(&["info", "skill/mine"]).stdout;
    assert!(
        info.contains("By Ada Lovelace, with Grace Hopper"),
        "{info}"
    );

    let ui = Ui::start(&s, "http://127.0.0.1:9");
    let (_, who) = ui.get("/api/whoami");
    assert_eq!(who["output"]["name"], "Ada Lovelace");
    let (_, srcs) = ui.get("/api/sources");
    let layer_of = |n: &str| {
        srcs["output"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == n)
            .unwrap()["layer"]
            .clone()
    };
    assert_eq!(layer_of("org-skills"), "org");
    assert_eq!(layer_of("ada-skills"), "user");

    // Clone down: a copy in your own source, with you as co-author.
    let (status, v) = ui.post(
        "/api/copy-item",
        json!({"key": "skill/write-spec", "from": "app-skills", "to": "ada-skills", "co_author": true}),
    );
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["output"]["moved"], false);
    assert_eq!(v["output"]["co_author_added"], "Ada Lovelace");
    let (_, copy) = ui.get("/api/source-item?source=ada-skills&key=skill%2Fwrite-spec");
    assert!(
        copy["output"]["text"]
            .as_str()
            .unwrap()
            .contains("co_authors: [\"Ada Lovelace\"]"),
        "{copy}"
    );
    assert_eq!(
        copy["output"]["meta"]["loadout"]["co_authors"],
        json!(["Ada Lovelace"])
    );
    let (_, orig) = ui.get("/api/source-item?source=app-skills&key=skill%2Fwrite-spec");
    assert!(
        !orig["output"]["text"].as_str().unwrap().is_empty(),
        "the original stays"
    );

    // Edit fields of the copy: only those keys change.
    let (status, v) = ui.post(
        "/api/source-item",
        json!({"source": "ada-skills", "key": "skill/write-spec", "fields": {"author": "Grace Hopper", "tags": ["specs"], "body": "My way.\n"}}),
    );
    assert_eq!(status, 200, "{v}");
    let (_, copy) = ui.get("/api/source-item?source=ada-skills&key=skill%2Fwrite-spec");
    let g = &copy["output"]["meta"]["loadout"];
    assert_eq!(g["author"], "Grace Hopper");
    assert_eq!(g["co_authors"], json!(["Ada Lovelace"]));
    assert_eq!(g["tags"], json!(["specs"]));
    assert_eq!(copy["output"]["body"], "My way.\n");
    let (status, _) = ui.post(
        "/api/source-item",
        json!({"source": "ada-skills", "key": "skill/write-spec", "fields": {"mode": "sometimes"}}),
    );
    assert_eq!(status, 400, "an invalid value is refused");

    // Something a broader layer locked can't be cloned down.
    let (status, v) = ui.post(
        "/api/copy-item",
        json!({"key": "skill/standards", "from": "org-skills", "to": "ada-skills"}),
    );
    assert_eq!(status, 400);
    assert!(v["error"].as_str().unwrap().contains("locked"), "{v}");

    // Promote: move a team item up to the org source.
    let (status, v) = ui.post(
        "/api/copy-item",
        json!({"key": "skill/extra", "from": "other-skills", "to": "org-skills", "remove": true}),
    );
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["output"]["moved"], true);
    let (_, up) = ui.get("/api/source-item?source=org-skills&key=skill%2Fextra");
    assert!(
        up["output"]["text"]
            .as_str()
            .unwrap()
            .contains("Opt-in extra.")
    );
    let (_, gone) = ui.get("/api/source-item?source=other-skills&key=skill%2Fextra");
    assert_eq!(gone["output"]["text"], "");
    let (status, _) = ui.post(
        "/api/copy-item",
        json!({"key": "skill/mine", "from": "ada-skills", "to": "ada-skills"}),
    );
    assert_eq!(status, 400, "same source");
}

/// A releases API whose latest release is v9.9.9.
fn releases() -> MockServer {
    MockServer::start(BTreeMap::from([(
        "/releases/latest".to_owned(),
        (200, r#"{"tag_name":"v9.9.9","assets":[]}"#.to_owned()),
    )]))
}

/// `/api/about` once the background lookup has answered (or after ~5 s).
fn about_when_checked(ui: &Ui) -> Value {
    for _ in 0..50 {
        let (status, v) = ui.get("/api/about");
        assert_eq!(status, 200, "{v}");
        if !v["output"]["checked_at"].is_null() {
            return v["output"].clone();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("the update check never finished");
}

#[test]
fn about_shows_the_version_and_a_newer_release() {
    let s = Sandbox::new();
    let github = MockServer::start(BTreeMap::new());
    let rel = releases();
    let ui = Ui::start_env(&s, github.url(), &[("LOADOUT_RELEASES_API", rel.url())]);
    let about = about_when_checked(&ui);
    assert_eq!(about["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(about["repo"], "https://github.com/Drewdl3/loadout");
    assert_eq!(about["check_for_updates"], true);
    assert_eq!(about["latest"], "9.9.9");
    assert_eq!(about["update_available"], true);
    let reqs = rel.requests();
    assert_eq!(reqs.len(), 1, "{reqs:?}");
    assert_eq!(
        reqs[0].accept.as_deref(),
        Some("application/vnd.github+json")
    );
    assert!(s.data.join("update-check.json").is_file());
    drop(ui);

    // Within a day, a restart reuses the answer instead of asking again.
    let ui = Ui::start_env(&s, github.url(), &[("LOADOUT_RELEASES_API", rel.url())]);
    assert_eq!(about_when_checked(&ui)["latest"], "9.9.9");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert_eq!(rel.requests().len(), 1);
}

#[test]
fn update_check_can_be_turned_off() {
    let s = Sandbox::new();
    std::fs::create_dir_all(&s.config).unwrap();
    std::fs::write(s.config.join("config.toml"), "check_for_updates = false\n").unwrap();
    let github = MockServer::start(BTreeMap::new());
    let rel = releases();
    let ui = Ui::start_env(&s, github.url(), &[("LOADOUT_RELEASES_API", rel.url())]);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let (status, v) = ui.get("/api/about");
    assert_eq!(status, 200, "{v}");
    let about = &v["output"];
    assert_eq!(about["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(about["check_for_updates"], false);
    assert!(about["latest"].is_null(), "{about}");
    assert_eq!(about["update_available"], false);
    assert!(rel.requests().is_empty(), "{:?}", rel.requests());
    assert!(!s.data.join("update-check.json").exists());
}
