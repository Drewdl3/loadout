//! Instantiating templates: filling `{{var}}` placeholders and
//! turning a template's frontmatter into a skill's.

use std::collections::{BTreeMap, BTreeSet};

/// A `{{ name }}` placeholder found in `text`: (byte range, name).
fn placeholders_in(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("{{").map(|i| i + from) {
        let Some(close) = text[open + 2..].find("}}").map(|i| i + open + 2) else {
            break;
        };
        let name = text[open + 2..close].trim();
        let valid = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if valid {
            out.push((open..close + 2, name));
            from = close + 2;
        } else {
            from = open + 2;
        }
    }
    out
}

/// Every placeholder name used in `text`.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    placeholders_in(text)
        .into_iter()
        .map(|(_, n)| n.to_owned())
        .collect()
}

/// Replaces each `{{ name }}` with its value. Placeholders without a value
/// are left as they are and returned.
pub fn render(text: &str, vars: &BTreeMap<String, String>) -> (String, BTreeSet<String>) {
    let mut out = String::with_capacity(text.len());
    let mut unknown = BTreeSet::new();
    let mut last = 0;
    for (range, name) in placeholders_in(text) {
        out.push_str(&text[last..range.start]);
        match vars.get(name) {
            Some(v) => out.push_str(v),
            None => {
                out.push_str(&text[range.clone()]);
                unknown.insert(name.to_owned());
            }
        }
        last = range.end;
    }
    out.push_str(&text[last..]);
    (out, unknown)
}

/// Rewrites a template's main file into the skill it produces: drops the
/// top-level `template:` block, sets `name:` to `name`, and records
/// `from_template: <provenance>` in the `loadout:` block (block or flow
/// style). `text` must start with YAML frontmatter.
pub fn instantiate_frontmatter(text: &str, name: &str, provenance: &str) -> Result<String, String> {
    let (front, body) = split(text).ok_or("the template's main file has no frontmatter")?;
    let mut lines: Vec<String> = Vec::new();
    let mut skipping = false;
    let mut in_loadout_block = false;
    let mut have_name = false;
    let mut have_loadout = false;
    let prov = format!("from_template: {}", yaml_str(provenance));
    for line in front.lines() {
        let top = !line.starts_with([' ', '\t']) && !line.trim().is_empty();
        if top {
            skipping = false;
            in_loadout_block = false;
        }
        if skipping {
            continue;
        }
        let key = top.then(|| line.split(':').next().unwrap_or("").trim());
        match key {
            Some("template") => {
                skipping = true;
                continue;
            }
            Some("name") => {
                lines.push(format!("name: {}", yaml_str(name)));
                have_name = true;
                continue;
            }
            Some("loadout") => {
                have_loadout = true;
                let rest = line.split_once(':').map_or("", |(_, r)| r.trim());
                if rest.is_empty() {
                    lines.push(line.to_owned());
                    in_loadout_block = true;
                    // Match the indentation of the block's first entry.
                    lines.push(format!("\u{0}{prov}"));
                } else if let Some(inner) = rest.strip_prefix('{').and_then(|r| r.strip_suffix('}'))
                {
                    let inner = inner.trim().trim_end_matches(',');
                    let sep = if inner.is_empty() { "" } else { ", " };
                    lines.push(format!("loadout: {{ {inner}{sep}{prov} }}"));
                } else {
                    return Err(format!("can't add provenance to `{line}`"));
                }
                continue;
            }
            _ => {}
        }
        if in_loadout_block
            && let Some(marker) = lines.iter().position(|l| l.starts_with('\u{0}'))
            && !line.trim().is_empty()
        {
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            lines[marker] = format!("{indent}{prov}");
        }
        lines.push(line.to_owned());
    }
    if let Some(marker) = lines.iter().position(|l| l.starts_with('\u{0}')) {
        lines[marker] = format!("  {prov}");
    }
    if !have_name {
        lines.insert(0, format!("name: {}", yaml_str(name)));
    }
    if !have_loadout {
        lines.push("loadout:".to_owned());
        lines.push(format!("  {prov}"));
    }
    Ok(format!("---\n{}\n---\n{body}", lines.join("\n")))
}

/// (frontmatter without delimiters, body after the closing line).
fn split(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let t = line.trim_end_matches(['\r', '\n']);
        if t == "---" || t == "..." {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

/// A double-quoted YAML scalar.
fn yaml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn renders_placeholders() {
        let (out, unknown) = render(
            "Hi {{team}}, use {{ host }} and {{ missing }}; not {{ 1x }} or {{}}.",
            &vars(&[("team", "Checkout"), ("host", "GitHub")]),
        );
        assert_eq!(
            out,
            "Hi Checkout, use GitHub and {{ missing }}; not {{ 1x }} or {{}}."
        );
        assert_eq!(unknown, BTreeSet::from(["missing".to_owned()]));
        assert_eq!(
            placeholders("{{a}} {{ b-c }} {{a}} {{ nope"),
            BTreeSet::from(["a".to_owned(), "b-c".to_owned()])
        );
        // Values are not re-scanned.
        assert_eq!(render("{{a}}", &vars(&[("a", "{{b}}")])).0, "{{b}}");
    }

    #[test]
    fn frontmatter_block_style() {
        let text = "---\nname: pr-workflow\ndescription: PRs for {{team}}.\ntemplate:\n  description: x\n  variables:\n    - { name: team }\nloadout:\n    tags: [git]\n---\n# Body\n";
        let out =
            instantiate_frontmatter(text, "checkout-prs", "eng:template/pr-workflow@abc").unwrap();
        assert_eq!(
            out,
            "---\nname: \"checkout-prs\"\ndescription: PRs for {{team}}.\nloadout:\n    from_template: \"eng:template/pr-workflow@abc\"\n    tags: [git]\n---\n# Body\n"
        );
    }

    #[test]
    fn frontmatter_flow_style_and_missing_keys() {
        let text = "---\ntemplate: { variables: [] }\nloadout: { tags: [a] }\n---\nx";
        assert_eq!(
            instantiate_frontmatter(text, "n", "p").unwrap(),
            "---\nname: \"n\"\nloadout: { tags: [a], from_template: \"p\" }\n---\nx"
        );
        let text = "---\ndescription: d\ntemplate:\n  variables: []\n---\n";
        assert_eq!(
            instantiate_frontmatter(text, "n", "p").unwrap(),
            "---\nname: \"n\"\ndescription: d\nloadout:\n  from_template: \"p\"\n---\n"
        );
        let text = "---\nloadout: {}\n---\n";
        assert_eq!(
            instantiate_frontmatter(text, "n", "p").unwrap(),
            "---\nname: \"n\"\nloadout: { from_template: \"p\" }\n---\n"
        );
        // An empty block-style loadout: gets a default indent.
        let text = "---\nloadout:\ndescription: d\n---\n";
        assert_eq!(
            instantiate_frontmatter(text, "n", "p").unwrap(),
            "---\nname: \"n\"\nloadout:\n  from_template: \"p\"\ndescription: d\n---\n"
        );
        assert!(instantiate_frontmatter("no frontmatter", "n", "p").is_err());
        assert!(instantiate_frontmatter("---\nloadout: [x]\n---\n", "n", "p").is_err());
    }
}
