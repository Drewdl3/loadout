//! Reading a checked-out source repo: `LOADOUT.md` and its items.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use loadout_model::frontmatter::Document;
use loadout_model::manifest::MANIFEST_FILE;
use loadout_model::{
    ItemId, ItemKey, ItemKind, ItemMeta, LoadoutBlock, Manifest, ManifestDoc, McpFrontmatter,
    McpServer,
};

/// Name of the file that marks a skill directory.
pub const SKILL_FILE: &str = "SKILL.md";

/// Name of the manifest that marks a plugin bundle directory.
pub const PLUGIN_FILE: &str = "PLUGIN.md";

/// A file inside an item, with a `/`-separated path relative to the item
/// root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemFile {
    pub path: String,
    pub contents: Vec<u8>,
}

/// One item found in a source.
#[derive(Debug, Clone)]
pub struct ScannedItem {
    pub id: ItemId,
    pub meta: ItemMeta,
    /// The item's `loadout:` block merged with the source defaults.
    pub loadout: LoadoutBlock,
    pub files: Vec<ItemFile>,
    pub content_hash: String,
    /// The server definition, for MCP items.
    pub mcp: Option<McpServer>,
}

/// A parsed source checkout.
#[derive(Debug, Clone)]
pub struct ScannedSource {
    pub manifest: Manifest,
    /// Effective default layer and group for the source's items.
    pub default_layer: String,
    pub default_group: Option<String>,
    pub items: Vec<ScannedItem>,
    pub warnings: Vec<String>,
}

/// Layer/group a subscription assigns to its source, overriding `LOADOUT.md`
/// defaults (items' own `loadout:` blocks still win).
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub layer: Option<String>,
    pub group: Option<String>,
}

/// Parses `LOADOUT.md` and all supported items under `root`.
pub fn scan_source(root: &Path) -> Result<ScannedSource> {
    scan_source_with(root, &Overrides::default())
}

/// [`scan_source`] with subscription overrides applied to the defaults.
pub fn scan_source_with(root: &Path, overrides: &Overrides) -> Result<ScannedSource> {
    let manifest_path = root.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("source has no readable {MANIFEST_FILE} at its root"))?;
    let manifest = ManifestDoc::parse(&text)
        .with_context(|| format!("invalid {}", manifest_path.display()))?
        .manifest;

    let mut defaults = manifest.defaults.clone();
    if let Some(layer) = &overrides.layer {
        defaults.layer = Some(layer.clone());
    }
    if let Some(group) = &overrides.group {
        defaults.group = Some(group.clone());
    }
    defaults.layer.get_or_insert_with(|| manifest.layer.clone());
    if defaults.group.is_none() {
        defaults.group.clone_from(&manifest.group);
    }

    let mut out = ScannedSource {
        default_layer: defaults.layer.clone().unwrap_or_default(),
        default_group: defaults.group.clone(),
        manifest,
        items: Vec::new(),
        warnings: Vec::new(),
    };
    scan_skills(root, &defaults, &mut out)?;
    scan_plugins(root, &defaults, &mut out)?;
    scan_mcp(root, &defaults, &mut out)?;
    scan_agents(root, &defaults, &mut out)?;
    scan_extras(root, &defaults, &mut out)?;
    out.items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn scan_skills(root: &Path, defaults: &LoadoutBlock, out: &mut ScannedSource) -> Result<()> {
    scan_dirs(root, defaults, out, ItemKind::Skill, SKILL_FILE)
}

/// Plugins: `plugins/<name>/` bundles with a `PLUGIN.md` manifest.
fn scan_plugins(root: &Path, defaults: &LoadoutBlock, out: &mut ScannedSource) -> Result<()> {
    scan_dirs(root, defaults, out, ItemKind::Plugin, PLUGIN_FILE)
}

/// Directory items: `<path>/<name>/` marked by `marker`.
fn scan_dirs(
    root: &Path,
    defaults: &LoadoutBlock,
    out: &mut ScannedSource,
    kind: ItemKind,
    marker: &str,
) -> Result<()> {
    let rel = out.manifest.paths.for_kind(kind).to_owned();
    let dir = root.join(&rel);
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ft = entry.file_type()?;
        if name.starts_with('.') || !ft.is_dir() {
            continue;
        }
        let item_dir = entry.path();
        if !item_dir.join(marker).is_file() {
            out.warnings
                .push(format!("{rel}/{name}: no {marker}, skipped"));
            continue;
        }
        match scan_dir_item(&out.manifest.name, kind, marker, &name, &item_dir, defaults) {
            Ok((item, warnings)) => {
                out.warnings
                    .extend(warnings.into_iter().map(|w| format!("{rel}/{name}: {w}")));
                out.items.push(item);
            }
            Err(e) => out.warnings.push(format!("{rel}/{name}: {e:#}, skipped")),
        }
    }
    Ok(())
}

fn scan_dir_item(
    source: &str,
    kind: ItemKind,
    marker: &str,
    name: &str,
    dir: &Path,
    defaults: &LoadoutBlock,
) -> Result<(ScannedItem, Vec<String>)> {
    let id = ItemId::new(source, ItemKey::new(kind, name)?)?;
    let (files, mut warnings) = read_tree(dir)?;
    let main = files
        .iter()
        .find(|f| f.path == marker)
        .with_context(|| format!("{marker} vanished"))?;
    let text =
        std::str::from_utf8(&main.contents).with_context(|| format!("{marker} is not UTF-8"))?;
    let meta: ItemMeta = Document::split(text)?.parse()?;
    if let Some(declared) = meta.name.as_deref()
        && declared != name
    {
        warnings.push(format!(
            "frontmatter name {declared:?} differs from directory name; using {name:?}"
        ));
    }
    let loadout = meta
        .loadout
        .clone()
        .unwrap_or_default()
        .or_defaults(defaults);
    let content_hash = loadout_core::hash::tree_hash(
        files
            .iter()
            .map(|f| (f.path.as_str(), f.contents.as_slice())),
    );
    Ok((
        ScannedItem {
            id,
            meta,
            loadout,
            files,
            content_hash,
            mcp: None,
        },
        warnings,
    ))
}

/// MCP servers: one `mcp/<name>.md` file each.
fn scan_mcp(root: &Path, defaults: &LoadoutBlock, out: &mut ScannedSource) -> Result<()> {
    let rel = out.manifest.paths.for_kind(ItemKind::Mcp).to_owned();
    scan_md_dir(root, &rel, defaults, out, |name| {
        Ok((ItemKey::new(ItemKind::Mcp, name)?, name.to_owned()))
    })
}

/// Subagents: one `agents/<name>.md` file each.
fn scan_agents(root: &Path, defaults: &LoadoutBlock, out: &mut ScannedSource) -> Result<()> {
    let rel = out.manifest.paths.for_kind(ItemKind::Agent).to_owned();
    scan_md_dir(root, &rel, defaults, out, |name| {
        Ok((ItemKey::new(ItemKind::Agent, name)?, name.to_owned()))
    })
}

/// Extras: `extras/<type>/<name>.md`, identified as `extra/<type>.<name>`.
fn scan_extras(root: &Path, defaults: &LoadoutBlock, out: &mut ScannedSource) -> Result<()> {
    let rel = out.manifest.paths.for_kind(ItemKind::Extra).to_owned();
    let dir = root.join(&rel);
    if !dir.is_dir() {
        return Ok(());
    }
    let mut types: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    types.sort_by_key(|e| e.file_name());
    for t in types {
        let ty = t.file_name().to_string_lossy().into_owned();
        if ty.starts_with('.') || !t.file_type()?.is_dir() {
            continue;
        }
        if let Err(e) = loadout_model::id::validate_item_name(&ty) {
            out.warnings.push(format!("{rel}/{ty}: {e}, skipped"));
            continue;
        }
        scan_md_dir(root, &format!("{rel}/{ty}"), defaults, out, |name| {
            Ok((
                ItemKey::new(ItemKind::Extra, format!("{ty}.{name}"))?,
                name.to_owned(),
            ))
        })?;
    }
    Ok(())
}

/// Scans `<rel>/*.md` (not `README.md` or hidden files); `key_of` turns a
/// file stem into the item key and the name the file is stored under.
fn scan_md_dir(
    root: &Path,
    rel: &str,
    defaults: &LoadoutBlock,
    out: &mut ScannedSource,
    key_of: impl Fn(&str) -> Result<(ItemKey, String)>,
) -> Result<()> {
    let dir = root.join(rel);
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = file.strip_suffix(".md") else {
            continue;
        };
        if file.starts_with('.') || file.eq_ignore_ascii_case("README.md") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            out.warnings
                .push(format!("{rel}/{file}: not a regular file, skipped"));
            continue;
        }
        let item = key_of(stem).and_then(|(key, stored)| {
            md_item(
                &out.manifest.name,
                key,
                stem,
                &stored,
                &entry.path(),
                defaults,
            )
        });
        match item {
            Ok((item, warnings)) => {
                out.warnings
                    .extend(warnings.into_iter().map(|w| format!("{rel}/{file}: {w}")));
                out.items.push(item);
            }
            Err(e) => out.warnings.push(format!("{rel}/{file}: {e:#}, skipped")),
        }
    }
    Ok(())
}

/// A single-file Markdown item (MCP server, subagent, extra). The file is
/// stored as `<stored>.md`; MCP items also get their server definition.
fn md_item(
    source: &str,
    key: ItemKey,
    stem: &str,
    stored: &str,
    path: &Path,
    defaults: &LoadoutBlock,
) -> Result<(ScannedItem, Vec<String>)> {
    let id = ItemId::new(source, key)?;
    let contents = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = std::str::from_utf8(&contents).context("not UTF-8")?;
    let doc = Document::split(text)?;
    let meta: ItemMeta = doc.parse()?;
    let mcp = if id.key().kind() == ItemKind::Mcp {
        Some(McpServer::from_frontmatter(
            &doc.parse::<McpFrontmatter>()?,
        )?)
    } else {
        None
    };
    let mut warnings = Vec::new();
    if let Some(declared) = meta.name.as_deref()
        && declared != stem
    {
        warnings.push(format!(
            "frontmatter name {declared:?} differs from file name; using {stem:?}"
        ));
    }
    let files = vec![ItemFile {
        path: format!("{stored}.md"),
        contents,
    }];
    let content_hash = loadout_core::hash::tree_hash(
        files
            .iter()
            .map(|f| (f.path.as_str(), f.contents.as_slice())),
    );
    let loadout = meta
        .loadout
        .clone()
        .unwrap_or_default()
        .or_defaults(defaults);
    Ok((
        ScannedItem {
            id,
            meta,
            loadout,
            files,
            content_hash,
            mcp,
        },
        warnings,
    ))
}

/// An item's main file inside its store directory: `SKILL.md`,
/// `PLUGIN.md`, or the single file of other kinds.
pub fn main_file(key: &ItemKey) -> String {
    match key.kind() {
        ItemKind::Skill => SKILL_FILE.to_owned(),
        ItemKind::Plugin => PLUGIN_FILE.to_owned(),
        _ => item_file_name(key),
    }
}

/// The file a single-file item (MCP server, subagent, extra) is stored as
/// inside its store directory.
pub fn item_file_name(key: &ItemKey) -> String {
    let name = key.name();
    let stored = match key.kind() {
        ItemKind::Extra => name.split_once('.').map_or(name, |(_, n)| n),
        _ => name,
    };
    format!("{stored}.md")
}

/// The file an MCP item is stored as inside its store directory.
pub fn mcp_file_name(name: &str) -> String {
    format!("{name}.md")
}

/// Reads every regular file under `dir`. Symlinks are skipped with a warning
/// so a source cannot smuggle files from outside its repo into the store.
pub fn read_tree(dir: &Path) -> Result<(Vec<ItemFile>, Vec<String>)> {
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(dir.to_path_buf(), String::new())];
    while let Some((abs, rel)) = stack.pop() {
        for entry in
            std::fs::read_dir(&abs).with_context(|| format!("reading {}", abs.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let ft = entry.file_type()?;
            if ft.is_symlink() {
                warnings.push(format!(
                    "{child_rel}: symlinks are not distributed, skipped"
                ));
            } else if ft.is_dir() {
                if name != ".git" {
                    stack.push((entry.path(), child_rel));
                }
            } else if ft.is_file() {
                let contents = std::fs::read(entry.path())
                    .with_context(|| format!("reading {}", entry.path().display()))?;
                files.push(ItemFile {
                    path: child_rel,
                    contents,
                });
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if files.is_empty() {
        bail!("no files");
    }
    Ok((files, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    const MANIFEST: &str = "---\nloadout: 1\nname: team-skills\nlayer: team\ngroup: payments-dev\ndefaults: { mode: default-off }\n---\n";

    #[test]
    fn scans_skills_with_defaults_and_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(r, "LOADOUT.md", MANIFEST);
        write(
            r,
            "skills/write-spec/SKILL.md",
            "---\nname: write-spec\ndescription: d\nloadout: { layer: squad, group: checkout }\n---\nbody",
        );
        write(r, "skills/write-spec/ref/notes.md", "notes");
        write(r, "skills/other/SKILL.md", "---\nname: renamed\n---\n");
        write(r, "skills/no-skill-md/README.md", "x");
        write(r, "skills/Bad_Name/SKILL.md", "---\n---\n");
        write(r, "skills/.hidden/SKILL.md", "---\n---\n");
        write(r, "skills/loose-file.md", "x");

        let s = scan_source(r).unwrap();
        let ids: Vec<String> = s.items.iter().map(|i| i.id.to_string()).collect();
        assert_eq!(
            ids,
            ["team-skills:skill/other", "team-skills:skill/write-spec"]
        );

        let ws = &s.items[1];
        assert_eq!(ws.loadout.layer.as_deref(), Some("squad"));
        assert_eq!(ws.loadout.group.as_deref(), Some("checkout"));
        assert_eq!(ws.loadout.mode, Some(loadout_model::Mode::DefaultOff));
        let paths: Vec<&str> = ws.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["SKILL.md", "ref/notes.md"]);

        let other = &s.items[0];
        assert_eq!(other.loadout.layer.as_deref(), Some("team"));
        assert_eq!(other.loadout.group.as_deref(), Some("payments-dev"));

        let w = s.warnings.join("\n");
        assert!(
            w.contains("skills/other: frontmatter name \"renamed\""),
            "{w}"
        );
        assert!(w.contains("skills/no-skill-md: no SKILL.md"), "{w}");
        assert!(w.contains("skills/Bad_Name: invalid item name"), "{w}");
        assert!(!w.contains(".hidden"), "{w}");
    }

    #[test]
    fn subscription_overrides_replace_source_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(r, "LOADOUT.md", MANIFEST);
        write(r, "skills/a/SKILL.md", "---\n---\n");
        write(
            r,
            "skills/b/SKILL.md",
            "---\nloadout: { group: own }\n---\n",
        );
        let s = scan_source_with(
            r,
            &Overrides {
                layer: Some("role".into()),
                group: Some("developer".into()),
            },
        )
        .unwrap();
        assert_eq!(s.default_layer, "role");
        assert_eq!(s.default_group.as_deref(), Some("developer"));
        assert_eq!(s.items[0].loadout.layer.as_deref(), Some("role"));
        assert_eq!(s.items[0].loadout.group.as_deref(), Some("developer"));
        assert_eq!(s.items[1].loadout.group.as_deref(), Some("own"));
    }

    #[test]
    fn content_hash_tracks_content() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(r, "LOADOUT.md", MANIFEST);
        write(r, "skills/a/SKILL.md", "---\n---\none");
        let h1 = scan_source(r).unwrap().items[0].content_hash.clone();
        assert_eq!(h1, scan_source(r).unwrap().items[0].content_hash);
        write(r, "skills/a/SKILL.md", "---\n---\ntwo");
        assert_ne!(h1, scan_source(r).unwrap().items[0].content_hash);
    }

    #[test]
    fn missing_or_bad_manifest_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = scan_source(tmp.path()).unwrap_err();
        assert!(format!("{err:#}").contains("LOADOUT.md"));
        write(
            tmp.path(),
            "LOADOUT.md",
            "---\nloadout: 9\nname: x\nlayer: team\n---\n",
        );
        assert!(scan_source(tmp.path()).is_err());
    }

    #[test]
    fn broken_frontmatter_skips_item() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(r, "LOADOUT.md", MANIFEST);
        write(r, "skills/a/SKILL.md", "---\nname: [\n---\n");
        let s = scan_source(r).unwrap();
        assert!(s.items.is_empty());
        assert!(
            s.warnings[0].contains("skills/a: invalid YAML"),
            "{:?}",
            s.warnings
        );
    }

    #[test]
    fn scans_mcp_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(r, "LOADOUT.md", MANIFEST);
        write(
            r,
            "mcp/jira.md",
            "---\nname: jira\ndescription: Jira\nloadout: { mode: required }\ncommand: npx\nenv: { JIRA_TOKEN: \"secret://jira/token\" }\n---\nbody\n",
        );
        write(r, "mcp/README.md", "# docs");
        write(r, "mcp/notes.txt", "x");
        write(r, "mcp/broken.md", "---\ndescription: no command\n---\n");
        let s = scan_source(r).unwrap();
        let ids: Vec<String> = s.items.iter().map(|i| i.id.to_string()).collect();
        assert_eq!(ids, ["team-skills:mcp/jira"]);
        let jira = &s.items[0];
        assert_eq!(jira.loadout.mode, Some(loadout_model::Mode::Required));
        assert_eq!(jira.loadout.group.as_deref(), Some("payments-dev"));
        assert_eq!(jira.files[0].path, "jira.md");
        assert_eq!(jira.mcp.as_ref().unwrap().refs().len(), 1);
        let w = s.warnings.join("\n");
        assert!(w.contains("mcp/broken.md: invalid MCP server"), "{w}");
        assert!(!w.contains("README"), "{w}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(r, "LOADOUT.md", MANIFEST);
        write(r, "skills/a/SKILL.md", "---\n---\n");
        write(r, "secret.txt", "s3cr3t");
        std::os::unix::fs::symlink(r.join("secret.txt"), r.join("skills/a/leak.txt")).unwrap();
        let s = scan_source(r).unwrap();
        assert_eq!(s.items[0].files.len(), 1);
        assert!(s.warnings.iter().any(|w| w.contains("leak.txt: symlinks")));
    }
}

/// A template found in a source.
#[derive(Debug, Clone)]
pub struct ScannedTemplate {
    /// `source:template/name`.
    pub id: String,
    pub source: String,
    pub name: String,
    pub front: loadout_model::template::TemplateFrontmatter,
    pub files: Vec<ItemFile>,
    /// The source checkout it was read from.
    pub root: PathBuf,
}

/// The templates under the source's `templates/` directory: each a
/// directory with a `SKILL.md` whose frontmatter may have a `template:`
/// block. Problems become warnings.
pub fn scan_templates(root: &Path) -> Result<(Vec<ScannedTemplate>, Vec<String>)> {
    let text = std::fs::read_to_string(root.join(MANIFEST_FILE))
        .with_context(|| format!("source has no readable {MANIFEST_FILE} at its root"))?;
    let manifest = ManifestDoc::parse(&text)?.manifest;
    let rel = manifest.paths.templates().to_owned();
    let dir = root.join(&rel);
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    if !dir.is_dir() {
        return Ok((out, warnings));
    }
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        let mut one = || -> Result<ScannedTemplate> {
            loadout_model::id::validate_item_name(&name)?;
            let (files, mut w) = read_tree(&entry.path())?;
            warnings.append(&mut w);
            let main = files
                .iter()
                .find(|f| f.path == SKILL_FILE)
                .with_context(|| format!("no {SKILL_FILE}"))?;
            let text = std::str::from_utf8(&main.contents)
                .with_context(|| format!("{SKILL_FILE} is not UTF-8"))?;
            let front: loadout_model::template::TemplateFrontmatter =
                Document::split(text)?.parse()?;
            front.template.validate().map_err(anyhow::Error::msg)?;
            Ok(ScannedTemplate {
                id: format!("{}:template/{name}", manifest.name),
                source: manifest.name.clone(),
                name: name.clone(),
                front,
                files,
                root: root.to_path_buf(),
            })
        };
        match one() {
            Ok(t) => {
                let declared: std::collections::BTreeSet<&str> = t
                    .front
                    .template
                    .variables
                    .iter()
                    .map(|v| v.name.as_str())
                    .collect();
                let used: std::collections::BTreeSet<String> = t
                    .files
                    .iter()
                    .filter_map(|f| std::str::from_utf8(&f.contents).ok())
                    .flat_map(loadout_core::template::placeholders)
                    .collect();
                for u in used.iter().filter(|u| !declared.contains(u.as_str())) {
                    warnings.push(format!(
                        "{rel}/{name}: {{{{{u}}}}} is used but not declared under template.variables"
                    ));
                }
                out.push(t);
            }
            Err(e) => warnings.push(format!("{rel}/{name}: {e:#}, skipped")),
        }
    }
    Ok((out, warnings))
}
