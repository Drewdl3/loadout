//! The TUI's state and key handling, independent of the terminal. Every
//! action is a `lo` command run through [`Loadout`], like the web UI.

use serde_json::Value;

/// Runs `lo <args> --json`; returns (exit code, JSON output).
pub trait Loadout {
    fn run(&self, args: &[&str]) -> (i32, Value);
    /// Synced sources, as the web UI's `/api/sources`.
    fn sources(&self) -> Value;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Catalog,
    Sources,
    Templates,
    Groups,
    Status,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Catalog,
        Tab::Sources,
        Tab::Templates,
        Tab::Groups,
        Tab::Status,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Catalog => "Catalog",
            Tab::Sources => "Sources",
            Tab::Templates => "Templates",
            Tab::Groups => "Groups",
            Tab::Status => "Status",
        }
    }
}

/// A key, reduced to what the app handles (the terminal layer maps
/// crossterm events onto it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    CtrlC,
}

/// A catalog filter: its name, the item field it reads, the key that
/// cycles it.
pub struct Facet {
    pub name: &'static str,
    pub key: char,
    field: fn(&Value) -> Vec<String>,
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub const FACETS: [Facet; 5] = [
    Facet {
        name: "state",
        key: 's',
        field: |d| vec![d["state"].as_str().unwrap_or_default().to_owned()],
    },
    Facet {
        name: "kind",
        key: 'k',
        field: |d| vec![d["kind"].as_str().unwrap_or_default().to_owned()],
    },
    Facet {
        name: "source",
        key: 'o',
        field: |d| vec![d["source"].as_str().unwrap_or_default().to_owned()],
    },
    Facet {
        name: "role",
        key: 'r',
        field: |d| {
            let r = strs(&d["roles"]);
            if r.is_empty() {
                vec!["(everyone)".to_owned()]
            } else {
                r
            }
        },
    },
    Facet {
        name: "tag",
        key: 't',
        field: |d| strs(&d["tags"]),
    },
];

/// What a text prompt is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prompt {
    Search,
    SourceUrl,
}

#[derive(Debug, Clone)]
pub struct Input {
    pub prompt: Prompt,
    pub text: String,
}

/// Filling in a template.
#[derive(Debug, Clone)]
pub struct Form {
    pub template: Value,
    /// (label, value, required); variables, then the skill name.
    pub fields: Vec<(String, String, bool)>,
    /// Destination sources; `dest` indexes it.
    pub dests: Vec<String>,
    pub dest: usize,
    /// The focused row: a field, or `fields.len()` for the destination.
    pub focus: usize,
}

pub struct App<'a> {
    loadout: Box<dyn Loadout + 'a>,
    pub tab: Tab,
    pub quit: bool,
    pub help: bool,
    /// (text, is an error).
    pub message: Option<(String, bool)>,
    pub input: Option<Input>,
    pub form: Option<Form>,
    // Catalog.
    pub items: Vec<Value>,
    pub query: String,
    pub all_sources: bool,
    pub filters: [Option<String>; 5],
    pub item_sel: usize,
    pub why: Option<Value>,
    // Sources.
    pub sources: Vec<(usize, Value)>,
    pub company_config: Option<String>,
    pub source_sel: usize,
    pub preview: Option<(String, Value)>,
    // Templates.
    pub templates: Vec<Value>,
    pub template_sel: usize,
    /// Last created skill's source, for `x` (export to a pull request).
    pub exportable: Option<(String, String)>,
    // Groups and status.
    pub groups: Vec<Value>,
    pub group_sel: usize,
    pub status: Value,
    pub pending_sel: usize,
}

impl<'a> App<'a> {
    pub fn new(loadout: Box<dyn Loadout + 'a>) -> App<'a> {
        let mut app = App {
            loadout,
            tab: Tab::Catalog,
            quit: false,
            help: false,
            message: None,
            input: None,
            form: None,
            items: Vec::new(),
            query: String::new(),
            all_sources: false,
            filters: Default::default(),
            item_sel: 0,
            why: None,
            sources: Vec::new(),
            company_config: None,
            source_sel: 0,
            preview: None,
            templates: Vec::new(),
            template_sel: 0,
            exportable: None,
            groups: Vec::new(),
            group_sel: 0,
            status: Value::Null,
            pending_sel: 0,
        };
        app.refresh();
        app
    }

    /// Reloads everything from `lo`.
    pub fn refresh(&mut self) {
        self.load_items();
        let (sources, company_config) = tree(&self.loadout.sources());
        self.sources = sources;
        self.company_config = company_config;
        self.templates = self.output(&["template", "list"])["templates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        self.groups = self.output(&["profile"])["groups"]
            .as_array()
            .map(|u| {
                u.iter()
                    .filter(|u| u["layer"] != "company")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.status = self.output(&["status"]);
        self.clamp();
    }

    fn load_items(&mut self) {
        let mut args = vec!["search", self.query.as_str(), "--limit", "0"];
        if self.all_sources {
            args.push("--all-sources");
        }
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.items = self.output(&args)["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        self.why = None;
    }

    /// The output of a read command (errors become the message).
    fn output(&mut self, args: &[&str]) -> Value {
        let (code, out) = self.loadout.run(args);
        if code != 0
            && let Some(e) = out["error"].as_str()
        {
            self.message = Some((e.to_owned(), true));
        }
        out
    }

    /// Runs a configuring command; reports and refreshes. True on success.
    fn act(&mut self, args: &[&str]) -> bool {
        let (code, out) = self.loadout.run(args);
        let ok = out.get("error").is_none() && (code == 0 || code == 2 || code == 4);
        let text = if let Some(e) = out["error"].as_str() {
            e.to_owned()
        } else {
            let warnings: Vec<&str> = out["warnings"]
                .as_array()
                .map(|w| w.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let mut t = format!("lo {}: done", args.join(" "));
            if code == 2 {
                t.push_str(" (changes wait for review: see Status)");
            }
            if let Some(w) = warnings.first() {
                t.push_str(&format!(" — {w}"));
            }
            t
        };
        self.message = Some((text, !ok));
        self.refresh();
        ok
    }

    /// Items passing the filters.
    pub fn visible(&self) -> Vec<&Value> {
        self.items
            .iter()
            .filter(|d| {
                FACETS
                    .iter()
                    .zip(&self.filters)
                    .all(|(f, sel)| sel.as_ref().is_none_or(|v| (f.field)(d).contains(v)))
            })
            .collect()
    }

    pub fn selected_item(&self) -> Option<&Value> {
        self.visible().get(self.item_sel).copied()
    }

    /// The values a facet can take among the items passing the *other*
    /// filters, sorted.
    fn facet_values(&self, i: usize) -> Vec<String> {
        let mut vals: Vec<String> = self
            .items
            .iter()
            .filter(|d| {
                FACETS
                    .iter()
                    .zip(&self.filters)
                    .enumerate()
                    .all(|(j, (f, sel))| {
                        j == i || sel.as_ref().is_none_or(|v| (f.field)(d).contains(v))
                    })
            })
            .flat_map(|d| (FACETS[i].field)(d))
            .filter(|v| !v.is_empty())
            .collect();
        vals.sort();
        vals.dedup();
        vals
    }

    fn cycle_filter(&mut self, i: usize) {
        let vals = self.facet_values(i);
        let next = match &self.filters[i] {
            None => vals.first().cloned(),
            Some(cur) => vals
                .iter()
                .position(|v| v == cur)
                .and_then(|p| vals.get(p + 1).cloned()),
        };
        self.filters[i] = next;
        self.item_sel = 0;
        self.why = None;
    }

    fn clamp(&mut self) {
        let n = self.visible().len();
        self.item_sel = self.item_sel.min(n.saturating_sub(1));
        self.source_sel = self.source_sel.min(self.sources.len().saturating_sub(1));
        self.template_sel = self
            .template_sel
            .min(self.templates.len().saturating_sub(1));
        self.group_sel = self.group_sel.min(self.groups.len().saturating_sub(1));
        let pending = self.status["pending"].as_array().map_or(0, Vec::len);
        self.pending_sel = self.pending_sel.min(pending.saturating_sub(1));
    }

    pub fn on_key(&mut self, key: Key) {
        if key == Key::CtrlC {
            self.quit = true;
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        if self.input.is_some() {
            self.on_input_key(key);
            return;
        }
        if self.form.is_some() {
            self.on_form_key(key);
            return;
        }
        match key {
            Key::Char('q') => self.quit = true,
            Key::Char('?') => self.help = true,
            Key::Tab => self.switch(1),
            Key::BackTab => self.switch(Tab::ALL.len() - 1),
            Key::Char(c @ '1'..='5') => self.tab = Tab::ALL[c as usize - '1' as usize],
            Key::Char('S') => {
                self.act(&["sync"]);
            }
            Key::Char('R') => {
                self.message = None;
                self.refresh();
            }
            _ => match self.tab {
                Tab::Catalog => self.on_catalog_key(key),
                Tab::Sources => self.on_sources_key(key),
                Tab::Templates => self.on_templates_key(key),
                Tab::Groups => self.on_groups_key(key),
                Tab::Status => self.on_status_key(key),
            },
        }
    }

    fn switch(&mut self, by: usize) {
        let i = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.tab = Tab::ALL[(i + by) % Tab::ALL.len()];
    }

    fn on_input_key(&mut self, key: Key) {
        let Some(input) = self.input.as_mut() else {
            return;
        };
        match key {
            Key::Esc => self.input = None,
            Key::Backspace => {
                input.text.pop();
            }
            Key::Char(c) => input.text.push(c),
            Key::Enter => {
                let input = self.input.take().expect("input");
                match input.prompt {
                    Prompt::Search => {
                        self.query = input.text.trim().to_owned();
                        self.item_sel = 0;
                        self.load_items();
                    }
                    Prompt::SourceUrl => {
                        let url = input.text.trim().to_owned();
                        if url.is_empty() {
                            return;
                        }
                        let (code, out) = self.loadout.run(&["subscribe", &url, "--dry-run"]);
                        if code == 0 && out.get("error").is_none() {
                            self.message = Some((
                                "Preview: press y to subscribe and sync, Esc to discard".to_owned(),
                                false,
                            ));
                            self.preview = Some((url, out));
                        } else {
                            let e = out["error"].as_str().unwrap_or("preview failed").to_owned();
                            self.message = Some((e, true));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn on_catalog_key(&mut self, key: Key) {
        let n = self.visible().len();
        match key {
            Key::Down | Key::Char('j') => {
                self.item_sel = (self.item_sel + 1).min(n.saturating_sub(1));
                self.why = None;
            }
            Key::Up => {
                self.item_sel = self.item_sel.saturating_sub(1);
                self.why = None;
            }
            Key::Char('/') => {
                self.input = Some(Input {
                    prompt: Prompt::Search,
                    text: self.query.clone(),
                });
            }
            Key::Char('c') => {
                self.filters = Default::default();
                self.item_sel = 0;
            }
            Key::Char('a') => {
                self.all_sources = !self.all_sources;
                self.item_sel = 0;
                self.load_items();
            }
            Key::Char(c) if FACETS.iter().any(|f| f.key == c) => {
                let i = FACETS.iter().position(|f| f.key == c).expect("facet");
                self.cycle_filter(i);
            }
            Key::Char('w') | Key::Enter => {
                if let Some(it) = self.selected_item() {
                    let key = format!(
                        "{}/{}",
                        it["kind"].as_str().unwrap_or_default(),
                        it["name"].as_str().unwrap_or_default()
                    );
                    let out = self.output(&["why", &key]);
                    self.why = Some(out);
                }
            }
            Key::Char(' ') | Key::Char('e') => {
                let Some(it) = self.selected_item().cloned() else {
                    return;
                };
                let id = it["id"].as_str().unwrap_or_default().to_owned();
                match it["state"].as_str() {
                    Some("enabled") => {
                        self.act(&["disable", &id]);
                    }
                    Some("disabled") => {
                        self.act(&["enable", &id]);
                    }
                    Some("shadowed") => {
                        self.act(&["prefer", &id]);
                    }
                    Some("not-subscribed") => {
                        let group = format!(
                            "{}:{}",
                            it["layer"].as_str().unwrap_or_default(),
                            it["group"].as_str().unwrap_or_default()
                        );
                        if self.act(&["join", &group]) {
                            self.act(&["sync"]);
                        }
                    }
                    _ => {
                        self.message = Some((
                            "Not for you: its group, role or target doesn't match (press w for why)"
                                .to_owned(),
                            true,
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    fn on_sources_key(&mut self, key: Key) {
        if let Some((url, _)) = &self.preview {
            match key {
                Key::Char('y') => {
                    let url = url.clone();
                    self.preview = None;
                    if self.act(&["subscribe", &url]) {
                        self.act(&["sync"]);
                    }
                }
                Key::Esc | Key::Char('n') => {
                    self.preview = None;
                    self.message = None;
                }
                _ => {}
            }
            return;
        }
        match key {
            Key::Down | Key::Char('j') => {
                self.source_sel = (self.source_sel + 1).min(self.sources.len().saturating_sub(1));
            }
            Key::Up => self.source_sel = self.source_sel.saturating_sub(1),
            Key::Char('n') | Key::Char('+') => {
                self.input = Some(Input {
                    prompt: Prompt::SourceUrl,
                    text: String::new(),
                });
            }
            Key::Char('u') => {
                let Some((_, s)) = self.sources.get(self.source_sel).cloned() else {
                    return;
                };
                if s["manual"] == true && s["via"].is_null() {
                    let url = s["url"].as_str().unwrap_or_default().to_owned();
                    if self.act(&["unsubscribe", &url]) {
                        self.act(&["sync"]);
                    }
                } else {
                    self.message = Some((
                        "Only your own subscriptions can be removed here (company config and upstream sources come with them)".to_owned(),
                        true,
                    ));
                }
            }
            Key::Enter => {
                if let Some((_, s)) = self.sources.get(self.source_sel) {
                    let name = s["name"].as_str().unwrap_or_default().to_owned();
                    self.filters = Default::default();
                    self.filters[2] = Some(name);
                    self.item_sel = 0;
                    self.tab = Tab::Catalog;
                }
            }
            _ => {}
        }
    }

    fn on_templates_key(&mut self, key: Key) {
        match key {
            Key::Down | Key::Char('j') => {
                self.template_sel =
                    (self.template_sel + 1).min(self.templates.len().saturating_sub(1));
            }
            Key::Up => self.template_sel = self.template_sel.saturating_sub(1),
            Key::Enter => {
                let Some(t) = self.templates.get(self.template_sel).cloned() else {
                    return;
                };
                let mut fields: Vec<(String, String, bool)> = t["variables"]
                    .as_array()
                    .map(|vs| {
                        vs.iter()
                            .map(|v| {
                                (
                                    v["name"].as_str().unwrap_or_default().to_owned(),
                                    v["default"].as_str().unwrap_or_default().to_owned(),
                                    v.get("default").is_none(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                fields.push((
                    "skill name".to_owned(),
                    t["name"].as_str().unwrap_or_default().to_owned(),
                    true,
                ));
                let own = t["source"].as_str().unwrap_or_default();
                let mut dests: Vec<String> = self
                    .sources
                    .iter()
                    .filter_map(|(_, s)| s["name"].as_str())
                    .filter(|n| *n != own)
                    .map(str::to_owned)
                    .collect();
                dests.push(own.to_owned());
                self.form = Some(Form {
                    template: t,
                    fields,
                    dests,
                    dest: 0,
                    focus: 0,
                });
            }
            Key::Char('x') => {
                if let Some((source, name)) = self.exportable.clone() {
                    let msg = format!("Add {name} from a template");
                    if self.act(&["export-source", &source, "--message", &msg]) {
                        self.exportable = None;
                    }
                }
            }
            _ => {}
        }
    }

    fn on_form_key(&mut self, key: Key) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let rows = form.fields.len() + 1;
        match key {
            Key::Esc => self.form = None,
            Key::Down | Key::Tab => form.focus = (form.focus + 1) % rows,
            Key::Up | Key::BackTab => form.focus = (form.focus + rows - 1) % rows,
            Key::Left if form.focus == form.fields.len() => {
                form.dest = (form.dest + form.dests.len() - 1) % form.dests.len().max(1);
            }
            Key::Right if form.focus == form.fields.len() => {
                form.dest = (form.dest + 1) % form.dests.len().max(1);
            }
            Key::Backspace if form.focus < form.fields.len() => {
                form.fields[form.focus].1.pop();
            }
            Key::Char(c) if form.focus < form.fields.len() => form.fields[form.focus].1.push(c),
            Key::Enter => {
                if form.focus + 1 < rows {
                    form.focus += 1;
                    return;
                }
                let form = self.form.take().expect("form");
                self.create_from_template(form);
            }
            _ => {}
        }
    }

    fn create_from_template(&mut self, form: Form) {
        let missing: Vec<&str> = form
            .fields
            .iter()
            .filter(|(_, v, req)| *req && v.trim().is_empty())
            .map(|(n, _, _)| n.as_str())
            .collect();
        if !missing.is_empty() {
            self.message = Some((format!("Fill in: {}", missing.join(", ")), true));
            self.form = Some(form);
            return;
        }
        let Some(dest) = form.dests.get(form.dest).cloned() else {
            self.message = Some((
                "No source to add it to; subscribe to one first".to_owned(),
                true,
            ));
            return;
        };
        let (vars, name) = form.fields.split_at(form.fields.len() - 1);
        let name = name[0].1.trim().to_owned();
        let mut args: Vec<String> = vec![
            "template".into(),
            "use".into(),
            form.template["id"].as_str().unwrap_or_default().into(),
            "--into".into(),
            dest.clone(),
            "--name".into(),
            name.clone(),
        ];
        for (k, v, _) in vars {
            args.push("--set".into());
            args.push(format!("{k}={v}"));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        if self.act(&args) {
            self.message = Some((
                format!(
                    "Created {dest}:skill/{name} in your working clone of {dest}. Press x to open a pull request."
                ),
                false,
            ));
            self.exportable = Some((dest, name));
        }
    }

    fn on_groups_key(&mut self, key: Key) {
        match key {
            Key::Down | Key::Char('j') => {
                self.group_sel = (self.group_sel + 1).min(self.groups.len().saturating_sub(1));
            }
            Key::Up => self.group_sel = self.group_sel.saturating_sub(1),
            Key::Enter | Key::Char(' ') => {
                let Some(u) = self.groups.get(self.group_sel).cloned() else {
                    return;
                };
                let id = format!(
                    "{}:{}",
                    u["layer"].as_str().unwrap_or_default(),
                    u["group"].as_str().unwrap_or_default()
                );
                if u["member"] == true {
                    self.act(&["leave", &id]);
                } else if u["joinable"] == true {
                    self.act(&["join", &id]);
                } else {
                    self.message = Some((
                        format!("{id} isn't opt-in; membership comes from its rule"),
                        true,
                    ));
                }
            }
            _ => {}
        }
    }

    fn on_status_key(&mut self, key: Key) {
        let pending: Vec<Value> = self.status["pending"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        match key {
            Key::Down | Key::Char('j') => {
                self.pending_sel = (self.pending_sel + 1).min(pending.len().saturating_sub(1));
            }
            Key::Up => self.pending_sel = self.pending_sel.saturating_sub(1),
            Key::Char('a') | Key::Enter => {
                if let Some(p) = pending.get(self.pending_sel) {
                    let source = p["source"].as_str().unwrap_or_default().to_owned();
                    self.act(&["approve", &source]);
                }
            }
            Key::Char('A') if !pending.is_empty() => {
                self.act(&["approve", "--all"]);
            }
            _ => {}
        }
    }
}

/// Sources as a tree: (depth, source), each upstream under the source that
/// pulled it in; and the company config URL.
fn tree(v: &Value) -> (Vec<(usize, Value)>, Option<String>) {
    let all: Vec<Value> = v["sources"].as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = all.iter().filter_map(|s| s["name"].as_str()).collect();
    let mut out = Vec::new();
    fn walk(
        all: &[Value],
        parent: Option<&str>,
        depth: usize,
        names: &[&str],
        out: &mut Vec<(usize, Value)>,
    ) {
        for s in all {
            let via = s["via"].as_str().filter(|v| names.contains(v));
            if via == parent && depth < 16 {
                out.push((depth, s.clone()));
                walk(all, s["name"].as_str(), depth + 1, names, out);
            }
        }
    }
    walk(&all, None, 0, &names, &mut out);
    (out, v["company_config"].as_str().map(str::to_owned))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;

    use super::*;

    #[derive(Clone, Default)]
    pub struct Fake {
        pub calls: Rc<RefCell<Vec<String>>>,
    }

    impl Loadout for Fake {
        fn run(&self, args: &[&str]) -> (i32, Value) {
            self.calls.borrow_mut().push(args.join(" "));
            let out = match args[0] {
                "search" => json!({"hits": [
                    {"id": "eng:skill/style", "kind": "skill", "name": "style", "source": "eng", "layer": "org", "group": "eng",
                     "state": "enabled", "tags": ["style"], "roles": ["developer"], "description": "House style."},
                    {"id": "eng:mcp/jira", "kind": "mcp", "name": "jira", "source": "eng", "layer": "org", "group": "eng",
                     "state": "disabled", "tags": ["jira"], "description": "Jira."},
                    {"id": "squad:skill/style", "kind": "skill", "name": "style", "source": "squad", "layer": "squad", "group": "checkout",
                     "state": "shadowed", "description": "Squad style."}
                ]}),
                "template" if args[1] == "list" => json!({"templates": [
                    {"id": "eng:template/pr-workflow", "source": "eng", "name": "pr-workflow",
                     "variables": [{"name": "team"}, {"name": "reviewers", "default": "2"}], "files": ["SKILL.md"]}
                ]}),
                "profile" => json!({"groups": [
                    {"layer": "company", "group": "acme", "member": true},
                    {"layer": "product", "group": "billing", "member": false, "joinable": true, "basis": "rule"}
                ]}),
                "status" => {
                    json!({"last_sync": "2026-09-23T10:00:00Z", "fingerprint": "lo-fp:AAAA-BBBB",
                    "enabled_items": 1, "sources": 2, "conflicts": false,
                    "pending": [{"source": "squad", "from": "aaaaaaaa", "to": "bbbbbbbb", "reason": "manual-source"}]})
                }
                "subscribe" if args.contains(&"--dry-run") => json!({"sources": [
                    {"name": "team", "layer": "team", "group": "t", "commit": "cccccccc", "items": [{"kind": "skill", "name": "x"}]}
                ], "warnings": []}),
                "why" => json!({"winner": "eng:skill/style", "enabled": true, "candidates": [
                    {"id": "eng:skill/style", "layer": "org", "group": "eng", "rank": 10, "outcome": "winner"}]}),
                _ => json!({"warnings": []}),
            };
            (0, out)
        }

        fn sources(&self) -> Value {
            json!({"sources": [
                {"name": "squad", "url": "/r/squad", "commit": "11111111", "manual": true},
                {"name": "eng", "url": "/r/eng", "commit": "22222222", "manual": true, "via": "squad"}
            ], "company_config": null})
        }
    }

    pub fn app() -> (App<'static>, Rc<RefCell<Vec<String>>>) {
        let fake = Fake::default();
        let calls = fake.calls.clone();
        (App::new(Box::new(fake)), calls)
    }

    fn keys(app: &mut App, ks: &[Key]) {
        for k in ks {
            app.on_key(*k);
        }
    }

    fn last(calls: &Rc<RefCell<Vec<String>>>, n: usize) -> Vec<String> {
        let c = calls.borrow();
        c[c.len().saturating_sub(n)..].to_vec()
    }

    #[test]
    fn filters_cycle_and_clear() {
        let (mut a, _) = app();
        assert_eq!(a.visible().len(), 3);
        a.on_key(Key::Char('k')); // kind: mcp
        assert_eq!(a.filters[1].as_deref(), Some("mcp"));
        assert_eq!(a.visible().len(), 1);
        a.on_key(Key::Char('k')); // kind: skill
        assert_eq!(a.visible().len(), 2);
        a.on_key(Key::Char('r')); // role: (everyone), among skills
        assert_eq!(a.filters[3].as_deref(), Some("(everyone)"));
        assert_eq!(a.visible()[0]["id"], "squad:skill/style");
        a.on_key(Key::Char('c'));
        assert_eq!(a.visible().len(), 3);
    }

    #[test]
    fn toggles_prefer_and_why_run_commands() {
        let (mut a, calls) = app();
        a.on_key(Key::Char(' ')); // enabled → disable
        assert!(
            calls
                .borrow()
                .contains(&"disable eng:skill/style".to_owned())
        );
        keys(&mut a, &[Key::Down, Key::Char(' ')]); // disabled → enable
        assert!(calls.borrow().contains(&"enable eng:mcp/jira".to_owned()));
        keys(&mut a, &[Key::Down, Key::Char(' ')]); // shadowed → prefer
        assert!(
            calls
                .borrow()
                .contains(&"prefer squad:skill/style".to_owned())
        );
        a.on_key(Key::Char('w'));
        assert!(a.why.is_some());
        keys(
            &mut a,
            &[Key::Char('/'), Key::Char('p'), Key::Char('r'), Key::Enter],
        );
        assert!(calls.borrow().contains(&"search pr --limit 0".to_owned()));
    }

    #[test]
    fn preview_then_subscribe_and_sync() {
        let (mut a, calls) = app();
        a.on_key(Key::Char('2'));
        assert_eq!(a.tab, Tab::Sources);
        assert_eq!(a.sources[1].0, 1, "eng is nested under squad");
        a.on_key(Key::Char('n'));
        for c in "/r/team".chars() {
            a.on_key(Key::Char(c));
        }
        a.on_key(Key::Enter);
        assert!(a.preview.is_some());
        assert!(
            calls
                .borrow()
                .contains(&"subscribe /r/team --dry-run".to_owned())
        );
        a.on_key(Key::Char('y'));
        let tail = calls.borrow().join("\n");
        assert!(tail.contains("subscribe /r/team\n"), "{tail}");
        assert!(tail.contains("\nsync\n"), "{tail}");
        // Upstream sources can't be unsubscribed directly.
        keys(&mut a, &[Key::Down, Key::Char('u')]);
        assert!(a.message.as_ref().unwrap().1);
    }

    #[test]
    fn template_form_builds_the_use_command() {
        let (mut a, calls) = app();
        a.on_key(Key::Char('3'));
        a.on_key(Key::Enter);
        // Submit from the destination row with `team` empty: refused.
        keys(&mut a, &[Key::Up, Key::Enter]);
        assert!(a.form.is_some());
        assert!(a.message.as_ref().unwrap().0.contains("team"));
        let form = a.form.as_mut().unwrap();
        form.focus = 0;
        for c in "Checkout".chars() {
            a.on_key(Key::Char(c));
        }
        keys(
            &mut a,
            &[Key::Enter, Key::Enter, Key::Enter, Key::Right, Key::Enter],
        );
        assert!(a.form.is_none());
        let cmd = last(&calls, 6).join("\n");
        assert!(
            cmd.contains(
                "template use eng:template/pr-workflow --into eng --name pr-workflow --set team=Checkout --set reviewers=2"
            ),
            "{cmd}"
        );
        a.on_key(Key::Char('x'));
        assert!(
            calls
                .borrow()
                .iter()
                .any(|c| c.starts_with("export-source eng --message"))
        );
    }

    #[test]
    fn groups_status_and_global_keys() {
        let (mut a, calls) = app();
        keys(&mut a, &[Key::Char('4'), Key::Enter]);
        assert!(calls.borrow().contains(&"join product:billing".to_owned()));
        keys(&mut a, &[Key::Char('5'), Key::Char('a')]);
        assert!(calls.borrow().contains(&"approve squad".to_owned()));
        a.on_key(Key::Char('S'));
        assert!(calls.borrow().contains(&"sync".to_owned()));
        keys(&mut a, &[Key::Tab]);
        assert_eq!(a.tab, Tab::Catalog);
        a.on_key(Key::Char('q'));
        assert!(a.quit);
    }
}
