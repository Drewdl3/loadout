//! Markdown-with-YAML-frontmatter documents (`LOADOUT.md`, `SKILL.md`, …).

use serde::de::DeserializeOwned;

use crate::ModelError;

/// A Markdown document split into its YAML frontmatter and body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Document<'a> {
    /// The YAML between the opening and closing `---` lines, if present.
    pub frontmatter: Option<&'a str>,
    /// Everything after the closing delimiter (or the whole text).
    pub body: &'a str,
}

impl<'a> Document<'a> {
    /// Splits `text` into frontmatter and body.
    ///
    /// Frontmatter is present only when the first line (after an optional
    /// BOM) is exactly `---`. It ends at the next line that is exactly `---`
    /// or `...`. A missing closing delimiter is an error. Both `\n` and
    /// `\r\n` line endings are accepted.
    pub fn split(text: &'a str) -> Result<Self, ModelError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let Some(rest) = strip_delimiter_line(text, "---") else {
            return Ok(Document {
                frontmatter: None,
                body: text,
            });
        };

        let mut offset = 0;
        for line in rest.split_inclusive('\n') {
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed == "---" || trimmed == "..." {
                return Ok(Document {
                    frontmatter: Some(&rest[..offset]),
                    body: &rest[offset + line.len()..],
                });
            }
            offset += line.len();
        }
        Err(ModelError::UnterminatedFrontmatter)
    }

    /// Deserializes the frontmatter into `T`. A document without frontmatter
    /// deserializes from an empty mapping.
    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, ModelError> {
        let yaml = match self.frontmatter {
            Some(y) if !y.trim().is_empty() => y,
            _ => "{}",
        };
        serde_saphyr::from_str(yaml).map_err(|e| ModelError::Yaml(e.to_string()))
    }
}

fn strip_delimiter_line<'a>(text: &'a str, delim: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(delim)?;
    rest.strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
        .or_else(|| rest.is_empty().then_some(rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn splits_frontmatter_and_body() {
        let doc = Document::split("---\nname: x\n---\n# Title\nbody\n").unwrap();
        assert_eq!(doc.frontmatter, Some("name: x\n"));
        assert_eq!(doc.body, "# Title\nbody\n");
    }

    #[test]
    fn handles_crlf_and_bom() {
        let doc = Document::split("\u{feff}---\r\nname: x\r\n---\r\nbody").unwrap();
        assert_eq!(doc.frontmatter, Some("name: x\r\n"));
        assert_eq!(doc.body, "body");
    }

    #[test]
    fn no_frontmatter_is_all_body() {
        let doc = Document::split("# Just markdown\n---\n").unwrap();
        assert_eq!(doc.frontmatter, None);
        assert_eq!(doc.body, "# Just markdown\n---\n");
    }

    #[test]
    fn dashes_must_be_alone_on_first_line() {
        let doc = Document::split("----\nx: 1\n---\n").unwrap();
        assert_eq!(doc.frontmatter, None);
    }

    #[test]
    fn accepts_dot_terminator_and_empty_frontmatter() {
        let doc = Document::split("---\nx: 1\n...\nbody").unwrap();
        assert_eq!(doc.frontmatter, Some("x: 1\n"));
        let doc = Document::split("---\n---\n").unwrap();
        assert_eq!(doc.frontmatter, Some(""));
        assert_eq!(doc.body, "");
        let parsed: BTreeMap<String, String> = doc.parse().unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn closing_delimiter_at_eof_without_newline() {
        let doc = Document::split("---\nx: 1\n---").unwrap();
        assert_eq!(doc.frontmatter, Some("x: 1\n"));
        assert_eq!(doc.body, "");
    }

    #[test]
    fn unterminated_is_error() {
        assert!(matches!(
            Document::split("---\nx: 1\n"),
            Err(ModelError::UnterminatedFrontmatter)
        ));
    }

    #[test]
    fn invalid_yaml_is_error() {
        let doc = Document::split("---\nx: [1, 2\n---\n").unwrap();
        assert!(matches!(
            doc.parse::<BTreeMap<String, serde_json::Value>>(),
            Err(ModelError::Yaml(_))
        ));
    }
}
