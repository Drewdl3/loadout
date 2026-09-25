//! Templates: `templates/<name>/SKILL.md` in a source, with a
//! `template:` block describing the variables its `{{var}}` placeholders
//! use. Templates are never installed; `lo template use` turns one into
//! a skill in another source.

use serde::{Deserialize, Serialize};

use crate::item::LoadoutBlock;

/// The frontmatter of a template's `SKILL.md`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TemplateFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub template: TemplateMeta,
    pub loadout: Option<LoadoutBlock>,
}

/// The `template:` block.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TemplateMeta {
    /// What the template is for, shown when choosing one (the skill's own
    /// `description` describes the generated skill).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The values to fill in.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<TemplateVar>,
}

/// One variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateVar {
    /// Used as `{{name}}` in the template's files.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Used when no value is given; a variable without one is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl TemplateVar {
    pub fn required(&self) -> bool {
        self.default.is_none()
    }
}

impl TemplateMeta {
    /// Checks variable names: `[A-Za-z_][A-Za-z0-9_-]*`, no duplicates.
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for v in &self.variables {
            let mut chars = v.name.chars();
            let ok = chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !ok {
                return Err(format!("invalid variable name {:?}", v.name));
            }
            if !seen.insert(&v.name) {
                return Err(format!("variable {:?} is declared twice", v.name));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::Document;

    #[test]
    fn parses_template_frontmatter() {
        let text = "---\nname: pr-workflow\ndescription: How {{team}} works with PRs.\ntemplate:\n  description: A PR workflow skill.\n  variables:\n    - { name: team, description: Your squad }\n    - { name: host, default: github }\nloadout: { tags: [git] }\n---\nBody\n";
        let fm: TemplateFrontmatter = Document::split(text).unwrap().parse().unwrap();
        assert_eq!(fm.name.as_deref(), Some("pr-workflow"));
        assert_eq!(fm.template.variables.len(), 2);
        assert!(fm.template.variables[0].required());
        assert!(!fm.template.variables[1].required());
        assert_eq!(fm.loadout.unwrap().tags, ["git"]);
        assert!(fm.template.validate().is_ok());
    }

    #[test]
    fn rejects_bad_variables() {
        let v = |n: &str| TemplateVar {
            name: n.into(),
            description: None,
            default: None,
        };
        for bad in [
            vec![v("1x")],
            vec![v("a b")],
            vec![v("x"), v("x")],
            vec![v("")],
        ] {
            let m = TemplateMeta {
                description: None,
                variables: bad,
            };
            assert!(m.validate().is_err(), "{m:?}");
        }
        let text = "---\ntemplate: { variables: [{ name: x, typo: 1 }] }\n---\n";
        assert!(
            Document::split(text)
                .unwrap()
                .parse::<TemplateFrontmatter>()
                .is_err()
        );
    }
}
