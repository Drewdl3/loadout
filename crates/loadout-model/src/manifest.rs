//! `LOADOUT.md` — the source manifest at the root of every source repo
//!.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ModelError;
use crate::frontmatter::Document;
use crate::id::{ItemKind, validate_source_name};
use crate::item::LoadoutBlock;
use crate::layer::Layer;

/// The only manifest schema version this build understands.
pub const MANIFEST_VERSION: u32 = 1;

/// File name of the manifest at a source repo root.
pub const MANIFEST_FILE: &str = "LOADOUT.md";

/// Parsed `LOADOUT.md` frontmatter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Manifest {
    /// Manifest schema version.
    pub loadout: u32,
    /// Stable source id, kebab-case.
    pub name: String,
    /// Default layer for items in this repo.
    pub layer: String,
    /// Suggested ranks for layers this source uses, e.g. its own
    /// `{ name: beta, rank: 27 }`. The company config's layers and your
    /// `config.toml` override them; when two sources disagree, the higher
    /// rank is used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,
    /// Default group. Required unless the company config maps this source to a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Defaults applied to every item in this source.
    #[serde(default)]
    pub defaults: LoadoutBlock,
    #[serde(default)]
    pub paths: SourcePaths,
    /// Higher sources this one builds on. Subscribing to this source also
    /// installs theirs, each at its own layer and group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream: Vec<Upstream>,
    /// Directories (relative to this manifest) searched for more
    /// `LOADOUT.md` files, e.g. `[squads]` in a team repo that holds a
    /// folder per squad. Each one found is a source of its own (own name,
    /// layer, group, defaults and paths) fetched and pinned with this repo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nested: Vec<String>,
    /// Company config block (only in the company config source), kept
    /// verbatim here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<crate::CompanyConfig>")]
    #[serde(rename = "company")]
    pub company_config: Option<Value>,
    /// Unknown keys, preserved.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// An entry of `upstream:`: a Git URL (or path), optionally with a ref and
/// where it sits for this source's subscribers.
/// A URL starting with `./` or `../` is relative to the declaring source's
/// own URL, like a Git submodule URL (`../eng-skills` is a sibling repo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Upstream {
    Url(String),
    Detailed {
        url: String,
        /// Branch, tag or commit to follow instead of the default branch.
        #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
        /// Display name for the upstream.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// Rank all its items at this layer, their own `layer:` included.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer: Option<String>,
        /// Rank all its items at exactly this rank.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rank: Option<i64>,
        /// Tie-breaker among equal-rank sources; higher wins.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<i64>,
    },
}

impl Upstream {
    pub fn url(&self) -> &str {
        match self {
            Upstream::Url(u) | Upstream::Detailed { url: u, .. } => u,
        }
    }

    pub fn git_ref(&self) -> Option<&str> {
        match self {
            Upstream::Url(_) => None,
            Upstream::Detailed { git_ref, .. } => git_ref.as_deref(),
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            Upstream::Url(_) => None,
            Upstream::Detailed { label, .. } => label.as_deref(),
        }
    }

    pub fn layer(&self) -> Option<&str> {
        match self {
            Upstream::Url(_) => None,
            Upstream::Detailed { layer, .. } => layer.as_deref(),
        }
    }

    pub fn rank(&self) -> Option<i64> {
        match self {
            Upstream::Url(_) => None,
            Upstream::Detailed { rank, .. } => *rank,
        }
    }

    pub fn priority(&self) -> Option<i64> {
        match self {
            Upstream::Url(_) => None,
            Upstream::Detailed { priority, .. } => *priority,
        }
    }

    /// The URL, with a relative one resolved against `base` (the declaring
    /// source's URL): each `..` drops one path segment of `base`.
    pub fn resolve(&self, base: &str) -> String {
        let url = self.url().trim();
        if !(url.starts_with("./") || url.starts_with("../")) {
            return url.to_owned();
        }
        let base = base.trim().trim_end_matches(['/', '\\']);
        let base = base.strip_suffix(".git").unwrap_or(base);
        let sep = if base.contains('\\') && !base.contains('/') {
            '\\'
        } else {
            '/'
        };
        let mut out = base.to_owned();
        for part in url.split('/') {
            match part {
                "" | "." => {}
                ".." => match out.rfind(['/', '\\']) {
                    Some(i) => out.truncate(i),
                    None => out.clear(),
                },
                p => {
                    out.push(sep);
                    out.push_str(p);
                }
            }
        }
        out
    }
}

/// Directories (relative to the repo root) holding each item kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SourcePaths {
    pub skills: String,
    pub mcp: String,
    pub agents: String,
    pub plugins: String,
    pub extras: String,
    /// Templates; never installed.
    pub templates: String,
}

impl Default for SourcePaths {
    fn default() -> Self {
        SourcePaths {
            skills: "skills/".into(),
            mcp: "mcp/".into(),
            agents: "agents/".into(),
            plugins: "plugins/".into(),
            extras: "extras/".into(),
            templates: "templates/".into(),
        }
    }
}

impl SourcePaths {
    /// The directory for `kind`, without a trailing slash.
    pub fn for_kind(&self, kind: ItemKind) -> &str {
        let p = match kind {
            ItemKind::Skill => &self.skills,
            ItemKind::Mcp => &self.mcp,
            ItemKind::Agent => &self.agents,
            ItemKind::Plugin => &self.plugins,
            ItemKind::Extra => &self.extras,
        };
        p.trim_end_matches('/')
    }

    /// The templates directory, without a trailing slash.
    pub fn templates(&self) -> &str {
        self.templates.trim_end_matches('/')
    }

    fn validate(&self) -> Result<(), ModelError> {
        let dirs = ItemKind::ALL
            .into_iter()
            .map(|k| (format!("{}s", k.as_str()), self.for_kind(k)))
            .chain([("templates".to_owned(), self.templates())]);
        for (field, p) in dirs {
            if !is_inner_path(p) {
                return Err(ModelError::InvalidManifest(format!(
                    "paths.{field} must be a relative path inside the repo, got {p:?}"
                )));
            }
        }
        Ok(())
    }
}

/// A `/`-separated relative path that stays inside the repo (a trailing
/// `/` is allowed).
fn is_inner_path(p: &str) -> bool {
    let p = p.trim_end_matches('/');
    !(p.is_empty()
        || p.starts_with('/')
        || p.contains('\\')
        || p.contains(':')
        || p.split('/').any(|c| c.is_empty() || c == "." || c == ".."))
}

/// A parsed `LOADOUT.md`: frontmatter plus the free-form Markdown body.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestDoc {
    pub manifest: Manifest,
    pub body: String,
}

impl ManifestDoc {
    /// Parses and validates the text of a `LOADOUT.md` file.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let doc = Document::split(text)?;
        if doc.frontmatter.is_none() {
            return Err(ModelError::InvalidManifest(
                "LOADOUT.md must start with YAML frontmatter".into(),
            ));
        }
        let manifest: Manifest = doc.parse()?;
        if manifest.loadout != MANIFEST_VERSION {
            return Err(ModelError::UnsupportedManifestVersion(manifest.loadout));
        }
        validate_source_name(&manifest.name)?;
        if manifest.layer.trim().is_empty() {
            return Err(ModelError::InvalidManifest(
                "layer must not be empty".into(),
            ));
        }
        manifest.paths.validate()?;
        if let Some(u) = manifest.upstream.iter().find(|u| u.url().trim().is_empty()) {
            return Err(ModelError::InvalidManifest(format!(
                "upstream entries need a URL, got {u:?}"
            )));
        }
        let mut seen = std::collections::BTreeSet::new();
        for l in &manifest.layers {
            if l.name.trim().is_empty() {
                return Err(ModelError::InvalidManifest(
                    "layers entries need a name".into(),
                ));
            }
            if !seen.insert(l.name.as_str()) {
                return Err(ModelError::InvalidManifest(format!(
                    "layer {:?} is listed twice in layers",
                    l.name
                )));
            }
        }
        if let Some(u) = manifest
            .upstream
            .iter()
            .find(|u| u.layer().is_some_and(|l| l.trim().is_empty()))
        {
            return Err(ModelError::InvalidManifest(format!(
                "upstream {} has an empty layer",
                u.url()
            )));
        }
        if let Some(p) = manifest.nested.iter().find(|p| !is_inner_path(p)) {
            return Err(ModelError::InvalidManifest(format!(
                "nested entries must be relative paths inside the repo, got {p:?}"
            )));
        }
        Ok(ManifestDoc {
            manifest,
            body: doc.body.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Mode;

    const EXAMPLE: &str = r#"---
loadout: 1
name: payments-skills
layer: team
group: payments-dev
owners: ["@acme/payments-leads"]
description: Skills and MCP config for the Payments dev team.
defaults:
  mode: default-on
paths:
  skills: skills/
  mcp: mcp/
  agents: agents/
  plugins: plugins/
  extras: extras/
---

# Payments team skills

Free-form docs for humans.
"#;

    #[test]
    fn parses_example() {
        let doc = ManifestDoc::parse(EXAMPLE).unwrap();
        let m = &doc.manifest;
        assert_eq!(m.name, "payments-skills");
        assert_eq!(m.layer, "team");
        assert_eq!(m.group.as_deref(), Some("payments-dev"));
        assert_eq!(m.owners, ["@acme/payments-leads"]);
        assert_eq!(m.defaults.mode, Some(Mode::DefaultOn));
        assert_eq!(m.paths.for_kind(ItemKind::Skill), "skills");
        assert!(doc.body.contains("# Payments team skills"));
    }

    #[test]
    fn minimal_manifest_uses_default_paths() {
        let doc = ManifestDoc::parse("---\nloadout: 1\nname: x\nlayer: team\n---\n").unwrap();
        assert_eq!(doc.manifest.paths, SourcePaths::default());
        assert_eq!(doc.manifest.group, None);
    }

    #[test]
    fn custom_paths() {
        let doc = ManifestDoc::parse(
            "---\nloadout: 1\nname: x\nlayer: team\npaths: { skills: ai/skills }\n---\n",
        )
        .unwrap();
        assert_eq!(doc.manifest.paths.for_kind(ItemKind::Skill), "ai/skills");
        assert_eq!(doc.manifest.paths.for_kind(ItemKind::Mcp), "mcp");
    }

    #[test]
    fn rejects_bad_manifests() {
        let cases = [
            ("no frontmatter", "# hi"),
            ("missing name", "---\nloadout: 1\nlayer: team\n---\n"),
            (
                "bad version",
                "---\nloadout: 2\nname: x\nlayer: team\n---\n",
            ),
            (
                "bad name",
                "---\nloadout: 1\nname: Not_Kebab\nlayer: team\n---\n",
            ),
            ("empty layer", "---\nloadout: 1\nname: x\nlayer: ''\n---\n"),
            (
                "escaping path",
                "---\nloadout: 1\nname: x\nlayer: team\npaths: { skills: ../../etc }\n---\n",
            ),
            (
                "absolute path",
                "---\nloadout: 1\nname: x\nlayer: team\npaths: { skills: /etc }\n---\n",
            ),
            (
                "escaping nested",
                "---\nloadout: 1\nname: x\nlayer: team\nnested: [../other]\n---\n",
            ),
            (
                "root as nested",
                "---\nloadout: 1\nname: x\nlayer: team\nnested: [.]\n---\n",
            ),
        ];
        for (what, text) in cases {
            assert!(
                ManifestDoc::parse(text).is_err(),
                "{what} should be rejected"
            );
        }
    }

    #[test]
    fn upstream_entries() {
        let doc = ManifestDoc::parse(
            "---\nloadout: 1\nname: x\nlayer: squad\nupstream:\n  - https://git.example.com/acme/eng-skills\n  - { url: ../team-skills, ref: stable }\n---\n",
        )
        .unwrap();
        let up = &doc.manifest.upstream;
        assert_eq!(up[0].url(), "https://git.example.com/acme/eng-skills");
        assert_eq!(up[0].git_ref(), None);
        assert_eq!(up[1].git_ref(), Some("stable"));
        let base = "https://git.example.com/acme/checkout-squad.git";
        assert_eq!(
            up[0].resolve(base),
            "https://git.example.com/acme/eng-skills"
        );
        assert_eq!(
            up[1].resolve(base),
            "https://git.example.com/acme/team-skills"
        );
        let local = Upstream::Url("../../shared/x".into());
        assert_eq!(local.resolve("/srv/git/acme/squad/"), "/srv/git/shared/x");
        assert_eq!(
            Upstream::Url("./sub".into()).resolve("/srv/repo"),
            "/srv/repo/sub"
        );
        assert_eq!(
            Upstream::Url("../eng".into()).resolve(r"C:\git\squad"),
            r"C:\git\eng"
        );
        assert!(
            ManifestDoc::parse("---\nloadout: 1\nname: x\nlayer: t\nupstream: ['']\n---\n")
                .is_err()
        );
    }

    #[test]
    fn layers_and_upstream_placement() {
        let doc = ManifestDoc::parse(
            "---\nloadout: 1\nname: x\nlayer: beta\nlayers: [{ name: beta, rank: 27 }]\nupstream:\n  - { url: ../eng, label: Engineering, layer: org, rank: 12, priority: 3 }\n---\n",
        )
        .unwrap();
        let m = &doc.manifest;
        assert_eq!(m.layers[0].name, "beta");
        assert_eq!(m.layers[0].rank, 27);
        let up = &m.upstream[0];
        assert_eq!(up.label(), Some("Engineering"));
        assert_eq!(up.layer(), Some("org"));
        assert_eq!(up.rank(), Some(12));
        assert_eq!(up.priority(), Some(3));
        assert!(
            ManifestDoc::parse(
                "---\nloadout: 1\nname: x\nlayer: t\nlayers: [{ name: t, rank: 1 }, { name: t, rank: 2 }]\n---\n"
            )
            .is_err()
        );
        assert!(
            ManifestDoc::parse(
                "---\nloadout: 1\nname: x\nlayer: t\nupstream: [{ url: ../y, layer: '' }]\n---\n"
            )
            .is_err()
        );
    }

    #[test]
    fn nested_directories() {
        let doc = ManifestDoc::parse(
            "---\nloadout: 1\nname: x\nlayer: team\nnested: [squads/, org/teams]\n---\n",
        )
        .unwrap();
        assert_eq!(doc.manifest.nested, ["squads/", "org/teams"]);
        let plain = ManifestDoc::parse("---\nloadout: 1\nname: x\nlayer: team\n---\n").unwrap();
        assert!(plain.manifest.nested.is_empty());
        assert!(!plain.manifest.extra.contains_key("nested"));
    }

    #[test]
    fn unsupported_version_error_names_version() {
        let err = ManifestDoc::parse("---\nloadout: 2\nname: x\nlayer: team\n---\n").unwrap_err();
        assert!(matches!(err, ModelError::UnsupportedManifestVersion(2)));
    }
}
