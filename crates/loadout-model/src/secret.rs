//! Secret references: URIs such as `secret://jira/token` that
//! repos distribute in place of secret values.
//!
//! A reference may be a whole value (`env://JIRA_TOKEN`) or embedded in a
//! larger string (`Bearer secret://jira/token`); [`Template`] splits a string
//! into literal text and references.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::ModelError;

/// URI schemes recognised as secret references.
pub const SCHEMES: [&str; 6] = ["secret", "env", "keychain", "op", "vault", "sops"];

/// A parsed secret reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecretRef {
    /// `secret://<name>`: resolved through the configured resolver chain.
    Named(String),
    /// `env://VAR_NAME`.
    Env(String),
    /// `keychain://service/account`.
    Keychain { service: String, account: String },
    /// `op://Vault/Item/field` (1Password CLI); kept as the full URI.
    Op(String),
    /// `vault://path/to/secret#field` (HashiCorp Vault KV).
    Vault { path: String, field: String },
    /// `sops://relative/path.enc.yaml#key`: a SOPS file inside the item's
    /// source repo.
    Sops { path: String, key: String },
}

impl SecretRef {
    pub fn scheme(&self) -> &'static str {
        match self {
            SecretRef::Named(_) => "secret",
            SecretRef::Env(_) => "env",
            SecretRef::Keychain { .. } => "keychain",
            SecretRef::Op(_) => "op",
            SecretRef::Vault { .. } => "vault",
            SecretRef::Sops { .. } => "sops",
        }
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretRef::Named(n) => write!(f, "secret://{n}"),
            SecretRef::Env(v) => write!(f, "env://{v}"),
            SecretRef::Keychain { service, account } => {
                write!(f, "keychain://{service}/{account}")
            }
            SecretRef::Op(rest) => write!(f, "op://{rest}"),
            SecretRef::Vault { path, field } => write!(f, "vault://{path}#{field}"),
            SecretRef::Sops { path, key } => write!(f, "sops://{path}#{key}"),
        }
    }
}

impl Serialize for SecretRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for SecretRef {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = |why: &str| ModelError::InvalidSecretRef(s.to_owned(), why.to_owned());
        let (scheme, rest) = s
            .split_once("://")
            .ok_or_else(|| bad("expected scheme://…"))?;
        if rest.is_empty() {
            return Err(bad("empty reference"));
        }
        if rest.chars().any(char::is_whitespace) {
            return Err(bad("references cannot contain whitespace"));
        }
        let split_hash = |what: &str| {
            rest.split_once('#')
                .filter(|(a, b)| !a.is_empty() && !b.is_empty())
                .ok_or_else(|| bad(&format!("expected {what}")))
        };
        match scheme {
            "secret" => {
                let ok = rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
                    && !rest.starts_with('/')
                    && !rest.ends_with('/');
                if !ok {
                    return Err(bad(
                        "secret names use letters, digits, '-', '_', '.' and inner '/'",
                    ));
                }
                Ok(SecretRef::Named(rest.to_owned()))
            }
            "env" => {
                if !is_env_name(rest) {
                    return Err(bad("expected an environment variable name"));
                }
                Ok(SecretRef::Env(rest.to_owned()))
            }
            "keychain" => {
                let (service, account) = rest
                    .split_once('/')
                    .filter(|(a, b)| !a.is_empty() && !b.is_empty())
                    .ok_or_else(|| bad("expected keychain://service/account"))?;
                Ok(SecretRef::Keychain {
                    service: service.to_owned(),
                    account: account.to_owned(),
                })
            }
            "op" => {
                if rest.split('/').filter(|p| !p.is_empty()).count() < 3 {
                    return Err(bad("expected op://vault/item/field"));
                }
                Ok(SecretRef::Op(rest.to_owned()))
            }
            "vault" => {
                let (path, field) = split_hash("vault://path#field")?;
                Ok(SecretRef::Vault {
                    path: path.to_owned(),
                    field: field.to_owned(),
                })
            }
            "sops" => {
                let (path, key) = split_hash("sops://relative/path#key")?;
                let escapes = path.starts_with('/')
                    || path.contains('\\')
                    || path.contains(':')
                    || path.split('/').any(|c| c.is_empty() || c == "..");
                if escapes {
                    return Err(bad("sops paths must be relative and stay inside the repo"));
                }
                Ok(SecretRef::Sops {
                    path: path.to_owned(),
                    key: key.to_owned(),
                })
            }
            _ => Err(bad("unknown scheme")),
        }
    }
}

/// Whether `s` is a portable environment variable name.
pub fn is_env_name(s: &str) -> bool {
    let mut b = s.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// A piece of a [`Template`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Lit(String),
    Ref(SecretRef),
}

/// A string value split into literal text and secret references.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Template {
    pub parts: Vec<Part>,
}

impl Template {
    /// Splits `s` at every `<scheme>://…` reference. A reference starts at a
    /// known scheme that is not preceded by a URI-scheme character (so
    /// `https://` never matches) and runs to the next whitespace or the end.
    pub fn parse(s: &str) -> Result<Self, ModelError> {
        let mut parts = Vec::new();
        let mut lit_start = 0;
        let mut i = 0;
        let bytes = s.as_bytes();
        while i < s.len() {
            let boundary = i == 0 || !is_scheme_char(bytes[i - 1]);
            let is_ref = boundary
                && SCHEMES
                    .iter()
                    .any(|sc| s[i..].starts_with(sc) && s[i + sc.len()..].starts_with("://"));
            if !is_ref {
                i += s[i..].chars().next().map_or(1, char::len_utf8);
                continue;
            }
            let end = s[i..]
                .find(char::is_whitespace)
                .map_or(s.len(), |off| i + off);
            if lit_start < i {
                parts.push(Part::Lit(s[lit_start..i].to_owned()));
            }
            parts.push(Part::Ref(s[i..end].parse()?));
            i = end;
            lit_start = end;
        }
        if lit_start < s.len() {
            parts.push(Part::Lit(s[lit_start..].to_owned()));
        }
        Ok(Template { parts })
    }

    pub fn has_refs(&self) -> bool {
        self.refs().next().is_some()
    }

    pub fn refs(&self) -> impl Iterator<Item = &SecretRef> {
        self.parts.iter().filter_map(|p| match p {
            Part::Ref(r) => Some(r),
            Part::Lit(_) => None,
        })
    }

    /// The literal value when the template contains no references.
    pub fn literal(&self) -> Option<String> {
        (!self.has_refs()).then(|| self.to_string())
    }

    /// The single reference when the whole value is one reference.
    pub fn whole_ref(&self) -> Option<&SecretRef> {
        match self.parts.as_slice() {
            [Part::Ref(r)] => Some(r),
            _ => None,
        }
    }

    /// Renders the template, replacing each reference with `f(ref)`.
    pub fn render<E>(
        &self,
        mut f: impl FnMut(&SecretRef) -> Result<String, E>,
    ) -> Result<String, E> {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Lit(l) => out.push_str(l),
                Part::Ref(r) => out.push_str(&f(r)?),
            }
        }
        Ok(out)
    }
}

impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for p in &self.parts {
            match p {
                Part::Lit(l) => f.write_str(l)?,
                Part::Ref(r) => write!(f, "{r}")?,
            }
        }
        Ok(())
    }
}

fn is_scheme_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_scheme_and_round_trips() {
        for s in [
            "secret://jira/token",
            "env://JIRA_TOKEN",
            "keychain://acme/jira",
            "op://Engineering/Jira/token",
            "vault://secret/data/jira#token",
            "sops://secrets/jira.enc.yaml#jira.token",
        ] {
            let r: SecretRef = s.parse().unwrap();
            assert_eq!(r.to_string(), s);
        }
        assert_eq!(
            "keychain://svc/acct/sub".parse::<SecretRef>().unwrap(),
            SecretRef::Keychain {
                service: "svc".into(),
                account: "acct/sub".into()
            }
        );
    }

    #[test]
    fn rejects_malformed_refs() {
        for s in [
            "secret://",
            "secret:///x",
            "secret://a b",
            "secret://a$b",
            "env://1ABC",
            "env://A-B",
            "keychain://svc",
            "op://vault/item",
            "vault://path",
            "vault://#f",
            "sops://../x.yaml#k",
            "sops:///etc/x#k",
            "sops://a//b#k",
            "ftp://x",
            "no-scheme",
        ] {
            assert!(s.parse::<SecretRef>().is_err(), "{s} should be rejected");
        }
    }

    #[test]
    fn template_splits_embedded_refs() {
        let t = Template::parse("Bearer secret://jira/token").unwrap();
        assert_eq!(
            t.parts,
            vec![
                Part::Lit("Bearer ".into()),
                Part::Ref(SecretRef::Named("jira/token".into()))
            ]
        );
        assert_eq!(t.to_string(), "Bearer secret://jira/token");
        assert!(t.whole_ref().is_none());

        let t = Template::parse("env://A and env://B").unwrap();
        assert_eq!(t.refs().count(), 2);

        let t = Template::parse("env://TOKEN").unwrap();
        assert_eq!(t.whole_ref(), Some(&SecretRef::Env("TOKEN".into())));
    }

    #[test]
    fn plain_urls_are_literals() {
        for s in [
            "https://acme.atlassian.net",
            "https://example.com/?next=menv://x",
            "",
            "héllo wörld",
        ] {
            let t = Template::parse(s).unwrap();
            assert_eq!(t.literal().as_deref(), Some(s), "{s}");
        }
    }

    #[test]
    fn invalid_embedded_ref_is_an_error() {
        assert!(Template::parse("Bearer env://not-valid").is_err());
    }

    #[test]
    fn render_substitutes() {
        let t = Template::parse("Bearer env://T x").unwrap();
        let out: Result<String, ()> = t.render(|r| Ok(format!("<{}>", r.scheme())));
        assert_eq!(out.unwrap(), "Bearer <env> x");
    }
}
