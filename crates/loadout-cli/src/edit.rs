//! Changing an item's frontmatter fields from the web UI's form, keeping
//! the rest of the file as the author wrote it.

use anyhow::{Context, Result, bail};
use loadout_core::frontmatter_edit::{self as fe, EditError};
use loadout_model::frontmatter::Document;
use loadout_model::{ItemMeta, LoadoutBlock};
use serde_json::Value;

fn meta(text: &str) -> Result<ItemMeta> {
    Ok(Document::split(text)?.parse::<ItemMeta>()?)
}

/// The item's own `loadout:` block (not filled in from source defaults).
fn loadout(text: &str) -> Result<LoadoutBlock> {
    Ok(meta(text)?.loadout.unwrap_or_default())
}

/// Sets or removes a key in the `loadout:` block, rewriting a flow-style
/// block in block style first.
fn set_loadout(text: &str, key: &str, value: Option<&Value>) -> Result<String> {
    let v = value.map(|v| v.to_string());
    match fe::set_loadout_key(text, key, v.as_deref()) {
        Ok(t) => Ok(t),
        Err(EditError::FlowLoadout) => {
            let Value::Object(map) = serde_json::to_value(loadout(text)?)? else {
                bail!("the loadout: block isn't a mapping");
            };
            let entries: Vec<(String, String)> =
                map.into_iter().map(|(k, v)| (k, v.to_string())).collect();
            let expanded = fe::expand_loadout(text, &entries);
            fe::set_loadout_key(&expanded, key, v.as_deref()).map_err(anyhow::Error::msg)
        }
        Err(e) => Err(anyhow::Error::msg(e)),
    }
}

/// Removes the other spellings of `co_authors`, so the file never has two.
fn drop_co_author_aliases(mut text: String) -> Result<String> {
    for alias in ["coAuthors", "coAuthor", "co_author"] {
        text = set_loadout(&text, alias, None)?;
    }
    Ok(text)
}

fn strings(v: &Value, what: &str) -> Result<Vec<String>> {
    let Some(a) = v.as_array() else {
        bail!("{what} must be a list");
    };
    a.iter()
        .map(|x| {
            x.as_str()
                .map(|s| s.trim().to_owned())
                .with_context(|| format!("{what} must be a list of strings"))
        })
        .filter(|s| s.as_ref().map_or(true, |s| !s.is_empty()))
        .collect()
}

/// Applies the form's `fields` to `text`. Each field is optional; a field
/// that's present replaces the current value, and an empty value removes
/// the key. Fields: `description`, `author`, `co_authors`, `tags`, `mode`,
/// `roles` (`applies_to.role`), `locked`, `body`.
pub fn apply_fields(text: &str, fields: &Value) -> Result<String> {
    let Some(fields) = fields.as_object() else {
        bail!("fields must be an object");
    };
    let mut text = text.to_owned();
    for (k, v) in fields {
        let str_or_remove = |v: &Value| -> Result<Option<Value>> {
            match v {
                Value::Null => Ok(None),
                Value::String(s) if s.trim().is_empty() => Ok(None),
                Value::String(s) => Ok(Some(Value::String(s.trim().to_owned()))),
                _ => bail!("{k} must be text"),
            }
        };
        let list_or_remove = |v: &Value| -> Result<Option<Value>> {
            let l = strings(v, k)?;
            Ok((!l.is_empty()).then(|| Value::from(l)))
        };
        text = match k.as_str() {
            "description" => {
                let v = str_or_remove(v)?.map(|v| v.to_string());
                fe::set_top_key(&text, "description", v.as_deref())
            }
            "author" | "mode" => set_loadout(&text, k, str_or_remove(v)?.as_ref())?,
            "tags" => set_loadout(&text, k, list_or_remove(v)?.as_ref())?,
            "co_authors" => {
                let t = drop_co_author_aliases(text)?;
                set_loadout(&t, k, list_or_remove(v)?.as_ref())?
            }
            "roles" => {
                let mut applies = loadout(&text)?.applies_to;
                match list_or_remove(v)? {
                    Some(_) => applies.insert("role".into(), strings(v, k)?),
                    None => applies.remove("role"),
                };
                let value = (!applies.is_empty())
                    .then(|| serde_json::to_value(&applies))
                    .transpose()?;
                set_loadout(&text, "applies_to", value.as_ref())?
            }
            "locked" => {
                let on = v
                    .as_bool()
                    .with_context(|| "locked must be true or false")?;
                set_loadout(&text, k, on.then_some(&Value::Bool(true)))?
            }
            "body" => fe::set_body(&text, v.as_str().context("body must be text")?),
            other => bail!("unknown field {other:?}"),
        };
    }
    // The result must still parse, with the values the form asked for.
    meta(&text).context("the edited frontmatter doesn't parse")?;
    Ok(text)
}

/// Adds `who` to `co_authors` unless they're already the author or a
/// co-author. Returns the new text and whether it changed.
pub fn add_co_author(text: &str, who: &str) -> Result<(String, bool)> {
    let who = who.trim();
    let g = loadout(text)?;
    let same = |a: &String| a.trim().eq_ignore_ascii_case(who);
    if who.is_empty() || g.author.iter().chain(&g.co_authors).any(same) {
        return Ok((text.to_owned(), false));
    }
    let mut list = g.co_authors;
    list.push(who.to_owned());
    let t = drop_co_author_aliases(text.to_owned())?;
    let t = set_loadout(&t, "co_authors", Some(&Value::from(list)))?;
    meta(&t)?;
    Ok((t, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SKILL: &str = "---\nname: x\ndescription: Old.\nallowed-tools: [Read]\nloadout:\n  mode: default-on # keep\n  applies_to:\n    role: [dev]\n    product: [billing]\n---\n# Body\n";

    #[test]
    fn form_fields_change_only_their_keys() {
        let t = apply_fields(
            SKILL,
            &json!({"description": "New. Use when asked.", "author": "Ada", "co_authors": ["Grace", " "], "tags": ["a"], "roles": [], "locked": true, "body": "Steps"}),
        )
        .unwrap();
        assert_eq!(
            t,
            "---\nname: x\ndescription: \"New. Use when asked.\"\nallowed-tools: [Read]\nloadout:\n  mode: default-on # keep\n  applies_to: {\"product\":[\"billing\"]}\n  author: \"Ada\"\n  co_authors: [\"Grace\"]\n  tags: [\"a\"]\n  locked: true\n---\nSteps\n"
        );
        let g = loadout(&t).unwrap();
        assert_eq!(g.author.as_deref(), Some("Ada"));
        assert!(!g.applies_to.contains_key("role"));
        // Empty values remove keys.
        let t = apply_fields(
            &t,
            &json!({"author": "", "locked": false, "mode": null, "tags": []}),
        )
        .unwrap();
        assert!(
            !t.contains("  author:")
                && !t.contains("locked")
                && !t.contains("mode")
                && !t.contains("tags"),
            "{t}"
        );
        assert!(apply_fields(SKILL, &json!({"mode": "sometimes"})).is_err());
        assert!(apply_fields(SKILL, &json!({"colour": "red"})).is_err());
    }

    #[test]
    fn co_authors_are_added_once_in_any_style() {
        let (t, changed) = add_co_author(SKILL, "Grace").unwrap();
        assert!(changed);
        assert!(t.contains("  co_authors: [\"Grace\"]\n"), "{t}");
        assert!(!add_co_author(&t, "grace").unwrap().1);
        let flow = "---\nname: x\nloadout: { author: Ada, coAuthors: [Linus] }\n---\n";
        assert!(!add_co_author(flow, "Ada").unwrap().1);
        let (t, _) = add_co_author(flow, "Grace").unwrap();
        assert_eq!(
            t,
            "---\nname: x\nloadout:\n  author: \"Ada\"\n  co_authors: [\"Linus\",\"Grace\"]\n---\n"
        );
    }
}
