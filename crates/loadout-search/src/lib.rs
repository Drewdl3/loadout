//! Item search: BM25-style ranking over name, description,
//! tags, product, role and body text, plus fuzzy matching on names. Pure:
//! callers build the documents and cache them.

use std::collections::{BTreeMap, BTreeSet};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where an item stands for this user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Enabled,
    Disabled,
    /// Another source's item with the same name wins.
    Shadowed,
    /// Not for this profile (group, `applies_to`, targets).
    NotForYou,
    /// From a company config source you're not subscribed to (`--all-sources`).
    NotSubscribed,
}

/// One searchable item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Doc {
    /// `source:kind/name`.
    pub id: String,
    pub kind: String,
    pub name: String,
    pub source: String,
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub product: Vec<String>,
    /// Roles from `applies_to.role`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub co_authors: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    pub state: State,
}

impl Doc {
    /// Whether `who` is the author or a co-author (case-insensitive,
    /// matching part of the name).
    pub fn wrote(&self, who: &str) -> bool {
        let who = who.to_lowercase();
        self.author
            .iter()
            .chain(&self.co_authors)
            .any(|a| a.to_lowercase().contains(&who))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub text: String,
    pub kind: Option<String>,
    pub tag: Option<String>,
    pub layer: Option<String>,
    /// Items for this role (`applies_to.role`); items for everyone match.
    pub role: Option<String>,
    pub group: Option<String>,
    /// Source id (`LOADOUT.md` name).
    pub source: Option<String>,
    /// Author or co-author (case-insensitive substring).
    pub author: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Hit {
    #[serde(flatten)]
    pub doc: Doc,
    pub score: f64,
}

/// Lowercase alphanumeric tokens of two or more characters; `write-spec`
/// yields `write` and `spec`.
pub fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(str::to_lowercase)
}

/// Field weights for BM25 (how much a term in that field counts).
const WEIGHTS: [(Field, f64); 5] = [
    (Field::Name, 4.0),
    (Field::Tags, 3.0),
    (Field::Description, 2.0),
    (Field::Meta, 1.5),
    (Field::Body, 1.0),
];

#[derive(Debug, Clone, Copy)]
enum Field {
    Name,
    Tags,
    Description,
    Meta,
    Body,
}

fn field_text(d: &Doc, f: Field) -> String {
    match f {
        Field::Name => d.name.clone(),
        Field::Tags => d.tags.join(" "),
        Field::Description => d.description.clone().unwrap_or_default(),
        Field::Meta => [
            d.product.join(" "),
            d.roles.join(" "),
            d.layer.clone(),
            d.author.clone().unwrap_or_default(),
            d.co_authors.join(" "),
        ]
        .join(" "),
        Field::Body => d.body.clone(),
    }
}

/// Runs `q` over `docs`: filters, then ranks by BM25 plus a fuzzy-name
/// bonus. An empty query lists every matching document by id.
pub fn search(docs: &[Doc], q: &Query) -> Vec<Hit> {
    let filtered: Vec<&Doc> = docs
        .iter()
        .filter(|d| q.kind.as_ref().is_none_or(|k| &d.kind == k))
        .filter(|d| q.layer.as_ref().is_none_or(|s| &d.layer == s))
        .filter(|d| {
            q.tag
                .as_ref()
                .is_none_or(|t| d.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
        })
        .filter(|d| {
            q.role
                .as_ref()
                .is_none_or(|r| d.roles.is_empty() || d.roles.contains(r))
        })
        .filter(|d| q.group.as_ref().is_none_or(|u| d.group.as_ref() == Some(u)))
        .filter(|d| q.source.as_ref().is_none_or(|s| &d.source == s))
        .filter(|d| q.author.as_ref().is_none_or(|a| d.wrote(a)))
        .collect();
    let limit = if q.limit == 0 { usize::MAX } else { q.limit };
    let terms: BTreeSet<String> = tokens(&q.text).collect();
    if q.text.trim().is_empty() {
        let mut hits: Vec<Hit> = filtered
            .into_iter()
            .map(|d| Hit {
                doc: d.clone(),
                score: 0.0,
            })
            .collect();
        hits.sort_by(|a, b| a.doc.id.cmp(&b.doc.id));
        hits.truncate(limit);
        return hits;
    }

    // Weighted term frequencies per document.
    let tf: Vec<BTreeMap<String, f64>> = filtered
        .iter()
        .map(|d| {
            let mut m: BTreeMap<String, f64> = BTreeMap::new();
            for (f, w) in WEIGHTS {
                for t in tokens(&field_text(d, f)) {
                    *m.entry(t).or_default() += w;
                }
            }
            m
        })
        .collect();
    let lens: Vec<f64> = tf.iter().map(|m| m.values().sum()).collect();
    let n = filtered.len() as f64;
    let avg = if lens.is_empty() {
        1.0
    } else {
        (lens.iter().sum::<f64>() / n).max(1.0)
    };
    let (k1, b) = (1.2, 0.75);

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(q.text.trim(), CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let fuzzy: Vec<Option<u32>> = filtered
        .iter()
        .map(|d| pattern.score(Utf32Str::new(&d.name, &mut buf), &mut matcher))
        .collect();
    let max_fuzzy = fuzzy.iter().flatten().copied().max().unwrap_or(1).max(1) as f64;

    let mut hits = Vec::new();
    for (i, d) in filtered.iter().enumerate() {
        let mut score = 0.0;
        for t in &terms {
            let Some(&f) = tf[i].get(t) else { continue };
            let df = tf.iter().filter(|m| m.contains_key(t)).count() as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            score += idf * f * (k1 + 1.0) / (f + k1 * (1.0 - b + b * lens[i] / avg));
        }
        if let Some(fz) = fuzzy[i] {
            score += 2.0 * fz as f64 / max_fuzzy;
        }
        if score > 0.0 {
            hits.push(Hit {
                doc: (*d).clone(),
                score: (score * 1000.0).round() / 1000.0,
            });
        }
    }
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.doc.id.cmp(&b.doc.id))
    });
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(id: &str, desc: &str, tags: &[&str], body: &str) -> Doc {
        let (source, rest) = id.split_once(':').unwrap();
        let (kind, name) = rest.split_once('/').unwrap();
        Doc {
            id: id.into(),
            kind: kind.into(),
            name: name.into(),
            source: source.into(),
            layer: "team".into(),
            group: Some("payments-dev".into()),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            product: vec![],
            roles: vec![],
            description: Some(desc.into()),
            author: None,
            co_authors: vec![],
            body: body.into(),
            state: State::Enabled,
        }
    }

    #[test]
    fn finds_and_filters_by_author() {
        let mut docs = corpus();
        docs[0].author = Some("Ada Lovelace".into());
        docs[1].co_authors = vec!["Grace Hopper <grace@example.com>".into()];
        let ids = |q: &Query| {
            search(&docs, q)
                .into_iter()
                .map(|h| h.doc.id)
                .collect::<Vec<_>>()
        };
        let by = |a: &str| Query {
            author: Some(a.into()),
            ..Default::default()
        };
        assert_eq!(ids(&by("ada")), [docs[0].id.clone()]);
        assert_eq!(ids(&by("GRACE")), [docs[1].id.clone()]);
        assert!(ids(&by("linus")).is_empty());
        let text = Query {
            text: "lovelace".into(),
            ..Default::default()
        };
        assert_eq!(ids(&text), [docs[0].id.clone()]);
    }

    fn corpus() -> Vec<Doc> {
        vec![
            doc(
                "pay:skill/write-spec",
                "Write a feature spec.",
                &["specs", "writing"],
                "Sections: problem, goals.",
            ),
            doc(
                "pay:skill/pci-checklist",
                "Check PCI concerns.",
                &["pci", "security"],
                "No card data in logs.",
            ),
            doc(
                "eng:mcp/jira",
                "Jira tickets and sprints.",
                &["jira", "tickets"],
                "Connects to Jira.",
            ),
            doc(
                "eng:skill/incident-runbook",
                "Incident response steps.",
                &["oncall"],
                "Declare the incident; write a postmortem spec.",
            ),
        ]
    }

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.doc.id.as_str()).collect()
    }

    #[test]
    fn ranks_name_and_tags_above_body() {
        let hits = search(
            &corpus(),
            &Query {
                text: "spec".into(),
                ..Query::default()
            },
        );
        assert_eq!(ids(&hits)[0], "pay:skill/write-spec");
        assert!(
            ids(&hits).contains(&"eng:skill/incident-runbook"),
            "body match counts"
        );
        let hits = search(
            &corpus(),
            &Query {
                text: "tickets".into(),
                ..Query::default()
            },
        );
        assert_eq!(ids(&hits), ["eng:mcp/jira"]);
    }

    #[test]
    fn fuzzy_names() {
        let hits = search(
            &corpus(),
            &Query {
                text: "pcichk".into(),
                ..Query::default()
            },
        );
        assert_eq!(ids(&hits), ["pay:skill/pci-checklist"]);
        let hits = search(
            &corpus(),
            &Query {
                text: "wrtspec".into(),
                ..Query::default()
            },
        );
        assert_eq!(ids(&hits)[0], "pay:skill/write-spec");
    }

    #[test]
    fn filters_and_limit() {
        let q = Query {
            text: String::new(),
            kind: Some("mcp".into()),
            ..Query::default()
        };
        assert_eq!(ids(&search(&corpus(), &q)), ["eng:mcp/jira"]);
        let q = Query {
            text: String::new(),
            tag: Some("PCI".into()),
            ..Query::default()
        };
        assert_eq!(ids(&search(&corpus(), &q)), ["pay:skill/pci-checklist"]);
        let q = Query {
            text: "spec".into(),
            limit: 1,
            ..Query::default()
        };
        assert_eq!(search(&corpus(), &q).len(), 1);
        let q = Query {
            text: String::new(),
            layer: Some("org".into()),
            ..Query::default()
        };
        assert!(search(&corpus(), &q).is_empty());
        assert!(
            search(
                &corpus(),
                &Query {
                    text: "zzzz".into(),
                    ..Query::default()
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn role_group_and_source_filters() {
        let mut docs = corpus();
        docs[1].roles = vec!["developer".into()];
        docs[0].roles = vec!["pm".into()];
        let q = |role: Option<&str>, group: Option<&str>, source: Option<&str>| Query {
            role: role.map(Into::into),
            group: group.map(Into::into),
            source: source.map(Into::into),
            ..Query::default()
        };
        // Items without a role limit are for every role.
        assert_eq!(
            ids(&search(&docs, &q(Some("developer"), None, None))),
            [
                "eng:mcp/jira",
                "eng:skill/incident-runbook",
                "pay:skill/pci-checklist"
            ]
        );
        assert_eq!(
            ids(&search(&docs, &q(None, None, Some("pay")))),
            ["pay:skill/pci-checklist", "pay:skill/write-spec"]
        );
        assert!(search(&docs, &q(None, Some("other"), None)).is_empty());
    }

    #[test]
    fn tokenizer() {
        let t: Vec<String> = tokens("Write-Spec: PCI/DSS a b").collect();
        assert_eq!(t, ["write", "spec", "pci", "dss"]);
    }
}
