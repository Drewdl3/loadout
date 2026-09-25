//! UI endpoints for organizing sources: read or edit an item in a working
//! clone, and copy or move an item between two sources (promoting it to a
//! broader layer, or cloning it down to customize it).

use anyhow::Context as _;
use loadout_git::Git;
use loadout_model::ItemKey;
use loadout_model::frontmatter::Document;
use serde_json::{Value, json};

use super::{Reply, json_reply};
use crate::ctx::Ctx;
use crate::state::Resolved;
use crate::work;

fn param<'a>(params: &'a [(String, String)], k: &str) -> Option<&'a str> {
    params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn fail(e: impl std::fmt::Display) -> Reply {
    json_reply(400, &json!({ "error": format!("{e:#}") }))
}

/// `GET /api/drafts`: for each source with a working clone, the items and
/// templates saved there but not yet proposed in a pull request. Never
/// clones: a source nobody has edited has nothing to show.
pub fn drafts(ctx: &Ctx) -> Reply {
    let run = || -> anyhow::Result<Value> {
        let Some(resolved) = Resolved::load(&ctx.paths.resolved_file())? else {
            return Ok(json!({ "sources": [] }));
        };
        let git = Git::new();
        let mut sources = Vec::new();
        for src in &resolved.sources {
            let dir = work::work_dir(ctx, &src.url);
            if !dir.join(".git").exists() {
                continue;
            }
            let entry = match work::changes(&git, &dir) {
                Ok(c) if c.is_empty() => continue,
                Ok(c) => json!({
                    "source": src.name,
                    "path": dir.display().to_string(),
                    "items": c.items.iter().map(|(k, ch)| json!({ "key": k.to_string(), "change": ch.as_str() })).collect::<Vec<_>>(),
                    "templates": c.templates.iter().map(|(n, ch)| json!({ "name": n, "change": ch.as_str() })).collect::<Vec<_>>(),
                    "other": c.other.iter().map(|(f, ch)| json!({ "path": f, "change": ch.as_str() })).collect::<Vec<_>>(),
                }),
                Err(e) => json!({
                    "source": src.name,
                    "path": dir.display().to_string(),
                    "error": format!("{e:#}"),
                }),
            };
            sources.push(entry);
        }
        Ok(json!({ "sources": sources }))
    };
    match run() {
        Ok(v) => json_reply(200, &json!({ "code": 0, "output": v })),
        Err(e) => fail(e),
    }
}

/// `GET /api/source-item?source=<name>&key=<kind/name>`: the item's main
/// file in the working clone (empty text if it doesn't exist yet).
pub fn read_item(ctx: &Ctx, params: &[(String, String)]) -> Reply {
    let (Some(source), Some(key)) = (param(params, "source"), param(params, "key")) else {
        return fail("?source=&key= required");
    };
    let run = || -> anyhow::Result<Value> {
        let key: ItemKey = key.parse()?;
        let w = work::open(ctx, &Git::new(), source)?;
        let (_, _, main) = work::item_path(&w.dir, &key)?;
        let text = std::fs::read_to_string(work::join(&w.dir, &main)).unwrap_or_default();
        // The parsed fields, for the form editor (null if they don't parse).
        let doc = Document::split(&text).ok();
        let meta = doc
            .as_ref()
            .and_then(|d| d.parse::<loadout_model::ItemMeta>().ok())
            .map(|m| json!({ "name": m.name, "description": m.description, "loadout": m.loadout.unwrap_or_default() }));
        let body = doc.map_or(text.as_str(), |d| d.body);
        Ok(json!({
            "source": source, "key": key.to_string(), "path": main, "text": text,
            "meta": meta, "body": body,
        }))
    };
    match run() {
        Ok(v) => json_reply(200, &json!({ "code": 0, "output": v })),
        Err(e) => fail(e),
    }
}

/// `POST /api/source-item {source, key, text}`: create or replace the
/// item's main file in the working clone. With `{source, key, fields}`
/// instead of `text`, changes only those fields of the existing file (see
/// [`crate::edit::apply_fields`]).
pub fn write_item(ctx: &Ctx, body: &Value) -> Reply {
    let (Some(source), Some(key)) = (body["source"].as_str(), body["key"].as_str()) else {
        return fail("{source, key, text} or {source, key, fields} required");
    };
    let run = || -> anyhow::Result<Value> {
        let key: ItemKey = key.parse()?;
        let w = work::open(ctx, &Git::new(), source)?;
        work::check_editable(ctx, &w.dir, &key)?;
        let (_, _, main) = work::item_path(&w.dir, &key)?;
        let path = work::join(&w.dir, &main);
        let text = match (body["text"].as_str(), body.get("fields")) {
            (Some(t), None) => t.to_owned(),
            (None, Some(fields)) => {
                let current = std::fs::read_to_string(&path)
                    .with_context(|| format!("{source} has no {key} to edit"))?;
                crate::edit::apply_fields(&current, fields)?
            }
            _ => anyhow::bail!("send either text or fields"),
        };
        // The file must stay readable by Loadout and the agents.
        Document::split(&text)?.parse::<loadout_model::ItemMeta>()?;
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        std::fs::write(&path, &text)?;
        Ok(json!({ "source": source, "key": key.to_string(), "path": main, "saved": true }))
    };
    match run() {
        Ok(v) => json_reply(200, &json!({ "code": 0, "output": v })),
        Err(e) => fail(e),
    }
}

/// `GET /api/source-template?source=<name>&name=<template>`: a template's
/// `SKILL.md` in the working clone (empty text if it doesn't exist yet).
pub fn read_template(ctx: &Ctx, params: &[(String, String)]) -> Reply {
    let (Some(source), Some(name)) = (param(params, "source"), param(params, "name")) else {
        return fail("?source=&name= required");
    };
    let run = || -> anyhow::Result<Value> {
        let w = work::open(ctx, &Git::new(), source)?;
        let main = work::template_path(&w.dir, name)?;
        let text = std::fs::read_to_string(work::join(&w.dir, &main)).unwrap_or_default();
        Ok(json!({ "source": source, "name": name, "path": main, "text": text }))
    };
    match run() {
        Ok(v) => json_reply(200, &json!({ "code": 0, "output": v })),
        Err(e) => fail(e),
    }
}

/// `POST /api/source-template {source, name, text, create?}`: write a
/// template's `SKILL.md` in the working clone. `create: true` refuses to
/// replace an existing template.
pub fn write_template(ctx: &Ctx, body: &Value) -> Reply {
    let (Some(source), Some(name), Some(text)) = (
        body["source"].as_str(),
        body["name"].as_str(),
        body["text"].as_str(),
    ) else {
        return fail("{source, name, text} required");
    };
    let run = || -> anyhow::Result<Value> {
        let warnings = check_template(name, text)?;
        let w = work::open(ctx, &Git::new(), source)?;
        let main = work::template_path(&w.dir, name)?;
        let path = work::join(&w.dir, &main);
        if body["create"].as_bool() == Some(true) && path.exists() {
            anyhow::bail!("{source} already has a template named {name}");
        }
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        std::fs::write(&path, text)?;
        Ok(
            json!({ "source": source, "name": name, "path": main, "saved": true, "warnings": warnings }),
        )
    };
    match run() {
        Ok(v) => json_reply(200, &json!({ "code": 0, "output": v })),
        Err(e) => fail(e),
    }
}

/// Checks a template the way `lo template use` will read it: the
/// `template:` block parses, every `{{placeholder}}` is declared, and
/// filling it in gives a valid skill. Returns warnings (unused variables).
pub fn check_template(name: &str, text: &str) -> anyhow::Result<Vec<String>> {
    use loadout_core::template::{instantiate_frontmatter, placeholders, render};
    use loadout_model::template::TemplateFrontmatter;

    loadout_model::id::validate_item_name(name)?;
    let fm: TemplateFrontmatter = Document::split(text)?.parse()?;
    fm.template.validate().map_err(anyhow::Error::msg)?;
    let declared: std::collections::BTreeSet<String> = fm
        .template
        .variables
        .iter()
        .map(|v| v.name.clone())
        .collect();
    let used = placeholders(text);
    let undeclared: Vec<String> = used
        .difference(&declared)
        .map(|v| format!("{{{{{v}}}}}"))
        .collect();
    if !undeclared.is_empty() {
        anyhow::bail!(
            "{} used in the text but not listed as a variable",
            undeclared.join(", ")
        );
    }
    let example = declared
        .iter()
        .map(|v| (v.clone(), "example".to_owned()))
        .collect();
    let (filled, _) = render(text, &example);
    let skill = instantiate_frontmatter(&filled, name, "check").map_err(anyhow::Error::msg)?;
    Document::split(&skill)?
        .parse::<loadout_model::ItemMeta>()
        .map_err(|e| anyhow::anyhow!("once filled in, the frontmatter isn't a valid skill: {e}"))?;
    Ok(declared
        .difference(&used)
        .map(|v| format!("variable {v:?} isn't used anywhere in the text"))
        .collect())
}

/// `POST /api/move-item {key, from, to}`: move an item's files from one
/// working clone to another (two edits; export each source).
pub fn move_item(ctx: &Ctx, body: &Value) -> Reply {
    let mut body = body.clone();
    body["remove"] = Value::Bool(true);
    copy_item(ctx, &body)
}

/// `POST /api/copy-item {key, from, to, remove?, co_author?}`: copy an
/// item's files from one source's working clone to another's. `remove`
/// deletes the original (a move; promoting usually wants this, since the
/// more specific copy would otherwise keep winning). `co_author` adds you
/// (Git `user.name`) to the copy's `co_authors`, for a copy you're about to
/// customize.
pub fn copy_item(ctx: &Ctx, body: &Value) -> Reply {
    let (Some(key), Some(from), Some(to)) = (
        body["key"].as_str(),
        body["from"].as_str(),
        body["to"].as_str(),
    ) else {
        return fail("{key, from, to} required");
    };
    let remove = body["remove"].as_bool() == Some(true);
    let co_author = body["co_author"].as_bool() == Some(true);
    let run = || -> anyhow::Result<Value> {
        if from == to {
            anyhow::bail!("{key} is already in {to}");
        }
        let key: ItemKey = key.parse()?;
        let git = Git::new();
        let a = work::open(ctx, &git, from)?;
        let b = work::open(ctx, &git, to)?;
        work::check_editable(ctx, &b.dir, &key)?;
        let (rel_a, is_dir, _) = work::item_path(&a.dir, &key)?;
        let (rel_b, _, _) = work::item_path(&b.dir, &key)?;
        let (src, dst) = (work::join(&a.dir, &rel_a), work::join(&b.dir, &rel_b));
        if !src.exists() {
            anyhow::bail!("{from} has no {key}");
        }
        if dst.exists() {
            anyhow::bail!("{to} already has {key}");
        }
        std::fs::create_dir_all(dst.parent().expect("parent"))?;
        if is_dir {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
        let (_, _, main) = work::item_path(&b.dir, &key)?;
        let mut warnings = Vec::new();
        let mut added = None;
        if co_author {
            match git_user(&git, Some(&b.dir)) {
                Some(who) => {
                    let file = work::join(&b.dir, &main);
                    let text = std::fs::read_to_string(&file)?;
                    let (text, changed) = crate::edit::add_co_author(&text, &who)?;
                    if changed {
                        std::fs::write(&file, text)?;
                        added = Some(who);
                    }
                }
                None => warnings.push(
                    "Git user.name isn't set, so you weren't added as a co-author".to_owned(),
                ),
            }
        }
        if remove {
            if is_dir {
                std::fs::remove_dir_all(&src)?;
            } else {
                std::fs::remove_file(&src)?;
            }
        }
        Ok(json!({
            "key": key.to_string(), "from": from, "to": to, "path": main,
            "moved": remove, "co_author_added": added, "warnings": warnings,
        }))
    };
    match run() {
        Ok(v) => json_reply(200, &json!({ "code": 0, "output": v })),
        Err(e) => fail(e),
    }
}

/// Git `user.name` (as seen from `dir`, which may set its own).
pub fn git_user(git: &Git, dir: Option<&std::path::Path>) -> Option<String> {
    git.run(dir, ["config", "user.name"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let d = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &d)?;
        } else {
            std::fs::copy(e.path(), &d)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_template;

    #[test]
    fn the_example_template_passes() {
        let text = include_str!("../../../../examples/eng-skills/templates/pr-workflow/SKILL.md");
        assert_eq!(
            check_template("pr-workflow", text).unwrap(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn template_problems_are_explained() {
        let t = |vars: &str, body: &str| {
            format!(
                "---\nname: x\ndescription: For {{{{team}}}}.\ntemplate:\n  variables: [{vars}]\n---\n{body}\n"
            )
        };
        let err = |name: &str, text: &str| check_template(name, text).unwrap_err().to_string();
        assert!(
            err("x", &t("{ name: team }", "Hi {{owner}}")).contains("{{owner}} used in the text")
        );
        assert!(err("x", &t("{ name: team }, { name: team }", "")).contains("declared twice"));
        assert!(err("Bad Name", &t("{ name: team }", "")).contains("Bad Name"));
        assert!(err("x", "no frontmatter").contains("frontmatter"));
        let warn =
            check_template("x", &t("{ name: team }, { name: extra, default: a }", "")).unwrap();
        assert_eq!(warn, ["variable \"extra\" isn't used anywhere in the text"]);
    }
}
