//! `LOADOUT.md` — the source manifest at the root of every source repo
//!.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ModelError;
use crate::frontmatter::Document;
use crate::id::{ItemKind, validate_source_name};
use crate::item::LoadoutBlock;

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

/// An entry of `upstream:`: a Git URL (or path), optionally with a ref.
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
            let bad = p.is_empty()
                || p.starts_with('/')
                || p.contains('\\')
                || p.contains(':')
                || p.split('/').any(|c| c.is_empty() || c == "." || c == "..");
            if bad {
                return Err(ModelError::InvalidManifest(format!(
                    "paths.{field} must be a relative path inside the repo, got {p:?}"
                )));
            }
        }
        Ok(())
    }
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
    fn unsupported_version_error_names_version() {
        let err = ManifestDoc::parse("---\nloadout: 2\nname: x\nlayer: team\n---\n").unwrap_err();
        assert!(matches!(err, ModelError::UnsupportedManifestVersion(2)));
    }
}
