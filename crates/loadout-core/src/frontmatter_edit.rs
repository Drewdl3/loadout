//! Editing single keys of a Markdown file's YAML frontmatter as text, so
//! everything else (comments, key order, other tools' keys, the body) stays
//! exactly as the author wrote it. Values are passed as YAML flow text
//! (JSON is valid YAML flow).

/// Why an edit couldn't be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The `loadout:` block is written in flow style (`loadout: { … }`); the
    /// caller rewrites it in block style with [`expand_loadout`] and retries.
    FlowLoadout,
    Other(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::FlowLoadout => f.write_str("the loadout: block is in flow style"),
            EditError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for EditError {}

/// A file split into frontmatter lines and body.
struct Parts {
    front: Vec<String>,
    body: String,
    /// Whether the file had frontmatter.
    had_front: bool,
}

fn parts(text: &str) -> Parts {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    if let Some(rest) = t
        .strip_prefix("---\n")
        .or_else(|| t.strip_prefix("---\r\n"))
    {
        let mut offset = 0;
        for line in rest.split_inclusive('\n') {
            let l = line.trim_end_matches(['\r', '\n']);
            if l == "---" || l == "..." {
                return Parts {
                    front: rest[..offset]
                        .lines()
                        .map(|l| l.trim_end_matches('\r').to_owned())
                        .collect(),
                    body: rest[offset + line.len()..].to_owned(),
                    had_front: true,
                };
            }
            offset += line.len();
        }
    }
    Parts {
        front: Vec::new(),
        body: text.to_owned(),
        had_front: false,
    }
}

fn join(p: &Parts) -> String {
    let mut out = String::from("---\n");
    for l in &p.front {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("---\n");
    if !p.had_front && !p.body.is_empty() && !p.body.starts_with('\n') {
        out.push('\n');
    }
    out.push_str(&p.body);
    out
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// The key of a mapping entry line (`key: …` / `key:`), after `indent`
/// spaces.
fn key_of(line: &str, indent: usize) -> Option<&str> {
    if indent_of(line) != indent {
        return None;
    }
    let rest = &line[indent..];
    if rest.starts_with(['#', '-']) {
        return None;
    }
    let (k, _) = rest.split_once(':')?;
    Some(k.trim().trim_matches(['"', '\'']))
}

/// Lines `start..end` belonging to the entry at `start` (indentation
/// `indent`): deeper-indented lines, and `- ` items at the same
/// indentation. Trailing blank lines are left out.
fn extent(lines: &[String], start: usize, indent: usize) -> usize {
    let mut end = start + 1;
    let mut last = start + 1;
    while end < lines.len() {
        let l = &lines[end];
        if l.trim().is_empty() {
            end += 1;
            continue;
        }
        let i = indent_of(l);
        if i > indent
            || (i == indent && l[i..].starts_with("- "))
            || (i == indent && &l[i..] == "-")
        {
            end += 1;
            last = end;
        } else {
            break;
        }
    }
    last
}

fn find(lines: &[String], from: usize, to: usize, indent: usize, key: &str) -> Option<usize> {
    (from..to).find(|&i| key_of(&lines[i], indent) == Some(key))
}

/// Sets (`Some(value)`) or removes (`None`) the top-level `key`.
pub fn set_top_key(text: &str, key: &str, value: Option<&str>) -> String {
    let mut p = parts(text);
    let n = p.front.len();
    match (find(&p.front, 0, n, 0, key), value) {
        (Some(i), Some(v)) => {
            let end = extent(&p.front, i, 0);
            p.front.splice(i..end, [format!("{key}: {v}")]);
        }
        (Some(i), None) => {
            let end = extent(&p.front, i, 0);
            p.front.drain(i..end);
        }
        (None, Some(v)) => {
            // After `name:`/`description:` when adding those, else at the end.
            let at = match key {
                "name" => 0,
                "description" => {
                    find(&p.front, 0, n, 0, "name").map_or(0, |i| extent(&p.front, i, 0))
                }
                _ => last_content(&p.front, 0, n),
            };
            p.front.insert(at, format!("{key}: {v}"));
        }
        (None, None) => return text.to_owned(),
    }
    join(&p)
}

/// One past the last non-blank line in `from..to`.
fn last_content(lines: &[String], from: usize, to: usize) -> usize {
    (from..to)
        .rev()
        .find(|&i| !lines[i].trim().is_empty())
        .map_or(from, |i| i + 1)
}

/// Sets or removes `key` inside the top-level `loadout:` block (block style;
/// a missing block is added).
pub fn set_loadout_key(text: &str, key: &str, value: Option<&str>) -> Result<String, EditError> {
    let mut p = parts(text);
    let n = p.front.len();
    let Some(g) = find(&p.front, 0, n, 0, "loadout") else {
        let Some(v) = value else {
            return Ok(text.to_owned());
        };
        let at = last_content(&p.front, 0, n);
        p.front
            .splice(at..at, ["loadout:".to_owned(), format!("  {key}: {v}")]);
        return Ok(join(&p));
    };
    let rest = p.front[g].split_once(':').map_or("", |(_, r)| r.trim());
    if !rest.is_empty() && !rest.starts_with('#') {
        return if rest == "null" || rest == "~" || rest == "{}" {
            p.front[g] = "loadout:".to_owned();
            set_loadout_key(&join(&p), key, value)
        } else if rest.starts_with('{') {
            Err(EditError::FlowLoadout)
        } else {
            Err(EditError::Other(format!("can't edit `{}`", p.front[g])))
        };
    }
    let end = extent(&p.front, g, 0);
    let child = (g + 1..end)
        .find(|&i| !p.front[i].trim().is_empty() && !p.front[i].trim_start().starts_with('#'))
        .map_or(2, |i| indent_of(&p.front[i]));
    let pad = " ".repeat(child);
    match (find(&p.front, g + 1, end, child, key), value) {
        (Some(i), Some(v)) => {
            let e = extent(&p.front, i, child);
            p.front.splice(i..e, [format!("{pad}{key}: {v}")]);
        }
        (Some(i), None) => {
            let e = extent(&p.front, i, child);
            p.front.drain(i..e);
        }
        (None, Some(v)) => {
            let at = last_content(&p.front, g + 1, end).max(g + 1);
            p.front.insert(at, format!("{pad}{key}: {v}"));
        }
        (None, None) => return Ok(text.to_owned()),
    }
    Ok(join(&p))
}

/// Rewrites a flow-style `loadout: { … }` as a block with `entries`
/// (key, YAML flow value): the parsed contents of the original.
pub fn expand_loadout(text: &str, entries: &[(String, String)]) -> String {
    let mut p = parts(text);
    let n = p.front.len();
    let Some(g) = find(&p.front, 0, n, 0, "loadout") else {
        return text.to_owned();
    };
    let end = extent(&p.front, g, 0);
    let mut block = vec!["loadout:".to_owned()];
    block.extend(entries.iter().map(|(k, v)| format!("  {k}: {v}")));
    p.front.splice(g..end, block);
    join(&p)
}

/// Replaces the body (everything after the frontmatter).
pub fn set_body(text: &str, body: &str) -> String {
    let mut p = parts(text);
    p.body = if body.is_empty() || body.ends_with('\n') {
        body.to_owned()
    } else {
        format!("{body}\n")
    };
    p.had_front = true;
    join(&p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str = "---\nname: x\n# a comment\ndescription: Old.\nallowed-tools: [Read]\nloadout:\n    mode: default-on   # keep\n    tags:\n    - a\n    - b\n    applies_to:\n      role: [dev]\n---\n# Body\n";

    #[test]
    fn replaces_and_removes_top_level_keys_keeping_the_rest() {
        let t = set_top_key(SKILL, "description", Some("\"New.\""));
        assert_eq!(
            t,
            SKILL.replace("description: Old.", "description: \"New.\"")
        );
        let t = set_top_key(SKILL, "allowed-tools", None);
        assert_eq!(t, SKILL.replace("allowed-tools: [Read]\n", ""));
        let t = set_top_key(
            "---\nname: x\nmetadata:\n  a: 1\n\n---\nB\n",
            "metadata",
            Some("{}"),
        );
        assert_eq!(t, "---\nname: x\nmetadata: {}\n\n---\nB\n");
        // Missing description goes after name.
        let t = set_top_key(
            "---\nname: x\nlicense: MIT\n---\n",
            "description",
            Some("D"),
        );
        assert_eq!(t, "---\nname: x\ndescription: D\nlicense: MIT\n---\n");
    }

    #[test]
    fn edits_the_loadout_block_at_its_own_indentation() {
        let t = set_loadout_key(SKILL, "author", Some("\"Ada\"")).unwrap();
        assert!(
            t.contains("      role: [dev]\n    author: \"Ada\"\n---\n"),
            "{t}"
        );
        let t = set_loadout_key(SKILL, "tags", Some("[\"c\"]")).unwrap();
        assert!(
            t.contains("    mode: default-on   # keep\n    tags: [\"c\"]\n    applies_to:"),
            "{t}"
        );
        let t = set_loadout_key(SKILL, "applies_to", None).unwrap();
        assert!(t.ends_with("    - b\n---\n# Body\n"), "{t}");
        // Unchanged elsewhere.
        assert!(t.starts_with("---\nname: x\n# a comment\ndescription: Old.\n"));
    }

    #[test]
    fn adds_a_loadout_block_or_reports_flow_style() {
        let t = set_loadout_key("---\nname: x\n---\nB\n", "author", Some("A")).unwrap();
        assert_eq!(t, "---\nname: x\nloadout:\n  author: A\n---\nB\n");
        let t = set_loadout_key("---\nloadout: {}\n---\n", "author", Some("A")).unwrap();
        assert_eq!(t, "---\nloadout:\n  author: A\n---\n");
        let flow = "---\nname: x\nloadout: { mode: required }\nz: 1\n---\n";
        assert_eq!(
            set_loadout_key(flow, "author", Some("A")),
            Err(EditError::FlowLoadout)
        );
        let t = expand_loadout(flow, &[("mode".into(), "\"required\"".into())]);
        assert_eq!(
            t,
            "---\nname: x\nloadout:\n  mode: \"required\"\nz: 1\n---\n"
        );
        assert!(
            set_loadout_key(&t, "author", Some("A"))
                .unwrap()
                .contains("  mode: \"required\"\n  author: A\nz: 1")
        );
    }

    #[test]
    fn replaces_the_body_and_handles_files_without_frontmatter() {
        assert_eq!(set_body(SKILL, "New"), SKILL.replace("# Body\n", "New\n"));
        assert_eq!(
            set_top_key("Just text\n", "name", Some("x")),
            "---\nname: x\n---\n\nJust text\n"
        );
    }
}
