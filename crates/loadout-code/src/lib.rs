//! Shortcodes and fingerprints: share an exact configuration
//! without a server, and check that two machines match.
//!
//! A shortcode is `lo1_` + Crockford base32(zstd(CBOR(payload))) + `_` +
//! the first 8 characters of Crockford base32(blake3(CBOR(payload))).
//! Pure: no I/O.

use std::collections::BTreeMap;
use std::io::Read;

use data_encoding::{Encoding, Specification};
use serde::{Deserialize, Serialize};

/// Shortcode prefix (format version 1).
pub const PREFIX: &str = "lo1_";
/// Payload schema version.
pub const PAYLOAD_VERSION: u32 = 1;
/// Fingerprint prefix.
pub const FP_PREFIX: &str = "lo-fp:";
/// Largest decompressed payload accepted (guards against zip bombs).
const MAX_PAYLOAD: u64 = 1 << 20;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodeError {
    #[error("not a Loadout shortcode (expected it to start with {PREFIX})")]
    Prefix,
    #[error("shortcode is malformed: {0}")]
    Malformed(String),
    #[error("shortcode checksum does not match (it was mistyped or cut off)")]
    Checksum,
    #[error("shortcode payload version {0} is not supported by this build")]
    Version(u32),
}

/// What a shortcode carries. Secrets are never part of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub v: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_config: Option<String>,
    /// Layer → groups.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profile: BTreeMap<String, Vec<String>>,
    /// Manual sources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourcePin>,
    /// Commits of company config-derived sources (the company config and group sources):
    /// URL → commit. Empty for a `--latest` code.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pins: BTreeMap<String, String>,
    /// Item id → enabled.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub toggles: BTreeMap<String, bool>,
    /// `kind/name` → preferred source (`lo prefer`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prefer: BTreeMap<String, String>,
    /// Enabled target ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
}

/// A manual source subscription with its pinned commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePin {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub priority: i64,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Crockford base32: `0-9A-Z` without `I L O U`; decoding ignores case and
/// reads `I`/`L` as `1` and `O` as `0`.
pub fn crockford() -> Encoding {
    let mut spec = Specification::new();
    spec.symbols.push_str("0123456789ABCDEFGHJKMNPQRSTVWXYZ");
    spec.translate.from.push_str("abcdefghjkmnpqrstvwxyzIiLlOo");
    spec.translate
        .to
        .push_str("ABCDEFGHJKMNPQRSTVWXYZ1111" /* I i L l */);
    spec.translate.to.push_str("00" /* O o */);
    spec.check_trailing_bits = false;
    spec.encoding()
        .expect("valid Crockford base32 specification")
}

fn cbor(payload: &Payload) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(payload, &mut out).expect("payload serializes");
    out
}

fn checksum(bytes: &[u8]) -> String {
    let full = crockford().encode(blake3::hash(bytes).as_bytes());
    full[..8].to_owned()
}

/// Encodes `payload` (its `v` is set to [`PAYLOAD_VERSION`]).
pub fn encode(payload: &Payload) -> String {
    let payload = Payload {
        v: PAYLOAD_VERSION,
        ..payload.clone()
    };
    let raw = cbor(&payload);
    let packed = ruzstd::encoding::compress_to_vec(
        raw.as_slice(),
        ruzstd::encoding::CompressionLevel::Fastest,
    );
    format!("{PREFIX}{}_{}", crockford().encode(&packed), checksum(&raw))
}

/// Decodes and verifies a shortcode. Surrounding whitespace, line breaks
/// and letter case are ignored.
pub fn decode(code: &str) -> Result<Payload, CodeError> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let rest = code
        .get(..PREFIX.len())
        .filter(|p| p.eq_ignore_ascii_case(PREFIX))
        .map(|_| &code[PREFIX.len()..])
        .ok_or(CodeError::Prefix)?;
    let (body, check) = rest
        .rsplit_once('_')
        .ok_or_else(|| CodeError::Malformed("missing checksum".into()))?;
    let enc = crockford();
    let packed = enc
        .decode(body.as_bytes())
        .map_err(|e| CodeError::Malformed(e.to_string()))?;
    let mut raw = Vec::new();
    ruzstd::decoding::StreamingDecoder::new(packed.as_slice())
        .map_err(|e| CodeError::Malformed(e.to_string()))?
        .take(MAX_PAYLOAD + 1)
        .read_to_end(&mut raw)
        .map_err(|e| CodeError::Malformed(e.to_string()))?;
    if raw.len() as u64 > MAX_PAYLOAD {
        return Err(CodeError::Malformed("payload too large".into()));
    }
    // Normalize the typed checksum the way the body is decoded.
    let normalized: String = check
        .chars()
        .map(|c| match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            u => u,
        })
        .collect();
    if normalized != checksum(&raw) {
        return Err(CodeError::Checksum);
    }
    let payload: Payload =
        ciborium::from_reader(raw.as_slice()).map_err(|e| CodeError::Malformed(e.to_string()))?;
    if payload.v != PAYLOAD_VERSION {
        return Err(CodeError::Version(payload.v));
    }
    Ok(payload)
}

/// The configuration fingerprint: over the resolved items
/// (id, content hash, enabled) and the target list, independent of order.
/// Two machines with the same fingerprint have identical agent
/// configuration.
pub fn fingerprint<'a>(
    items: impl IntoIterator<Item = (&'a str, &'a str, bool)>,
    targets: impl IntoIterator<Item = &'a str>,
) -> String {
    let mut items: Vec<(&str, &str, bool)> = items.into_iter().collect();
    items.sort();
    items.dedup();
    let mut targets: Vec<&str> = targets.into_iter().collect();
    targets.sort_unstable();
    targets.dedup();
    let mut h = blake3::Hasher::new();
    h.update(b"loadout-fingerprint-v1\0");
    let mut field = |s: &str| {
        h.update(&(s.len() as u64).to_le_bytes());
        h.update(s.as_bytes());
    };
    for (id, hash, enabled) in &items {
        field(id);
        field(hash);
        field(if *enabled { "on" } else { "off" });
    }
    field("--targets--");
    for t in &targets {
        field(t);
    }
    let code = crockford().encode(h.finalize().as_bytes());
    format!("{FP_PREFIX}{}-{}", &code[..4], &code[4..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample() -> Payload {
        Payload {
            v: PAYLOAD_VERSION,
            company_config: Some("https://git.example.com/acme/company-config".into()),
            profile: [
                ("company".to_string(), vec!["acme".to_string()]),
                ("team".to_string(), vec!["payments-dev".to_string()]),
            ]
            .into(),
            sources: vec![SourcePin {
                url: "https://git.example.com/someone/extra".into(),
                commit: Some("3f9c2a1b4d5e6f708192a3b4c5d6e7f8091a2b3c".into()),
                layer: Some("role".into()),
                group: Some("developer".into()),
                priority: 1,
            }],
            pins: [(
                "https://git.example.com/acme/company-config".to_string(),
                "0123456789abcdef0123456789abcdef01234567".to_string(),
            )]
            .into(),
            toggles: [("eng-skills:skill/incident-runbook".to_string(), true)].into(),
            prefer: BTreeMap::new(),
            targets: vec!["claude-code".into()],
        }
    }

    #[test]
    fn round_trips() {
        let p = sample();
        let code = encode(&p);
        assert!(code.starts_with("lo1_"), "{code}");
        assert_eq!(decode(&code).unwrap(), p);
        // Deterministic.
        assert_eq!(encode(&p), code);
    }

    #[test]
    fn tolerant_of_case_whitespace_and_ambiguous_letters() {
        let code = encode(&sample());
        let body = &code[PREFIX.len()..];
        let messy = format!(
            "  LO1_{}\n",
            body.to_lowercase().replace('1', "l").replace('0', "o")
        );
        assert_eq!(decode(&messy).unwrap(), sample());
        let wrapped: String = code
            .chars()
            .enumerate()
            .flat_map(|(i, c)| if i % 40 == 39 { vec![c, '\n'] } else { vec![c] })
            .collect();
        assert_eq!(decode(&wrapped).unwrap(), sample());
    }

    #[test]
    fn detects_damage() {
        let code = encode(&sample());
        assert_eq!(decode("xyz"), Err(CodeError::Prefix));
        assert!(matches!(decode("lo1_ABC"), Err(CodeError::Malformed(_))));
        // A wrong checksum.
        let (body, check) = code.rsplit_once('_').unwrap();
        let bad_check = format!(
            "{body}_{}",
            if &check[..1] == "A" { "B" } else { "A" }.to_owned() + &check[1..]
        );
        assert_eq!(decode(&bad_check), Err(CodeError::Checksum));
        // A damaged body.
        let mut chars: Vec<char> = code.chars().collect();
        let i = 10;
        chars[i] = if chars[i] == 'Z' { 'Y' } else { 'Z' };
        let damaged: String = chars.into_iter().collect();
        assert!(decode(&damaged).is_err());
        // Truncated.
        assert!(decode(&code[..code.len() - 12]).is_err());
    }

    #[test]
    fn rejects_other_versions() {
        let raw = cbor(&Payload {
            v: 2,
            ..Payload::default()
        });
        let packed = ruzstd::encoding::compress_to_vec(
            raw.as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        let code = format!("{PREFIX}{}_{}", crockford().encode(&packed), checksum(&raw));
        assert_eq!(decode(&code), Err(CodeError::Version(2)));
    }

    #[test]
    fn fingerprint_shape_and_sensitivity() {
        let items = [
            ("a:skill/x", "blake3:1", true),
            ("b:mcp/y", "blake3:2", false),
        ];
        let fp = fingerprint(items, ["claude-code"]);
        assert!(
            fp.starts_with("lo-fp:") && fp.len() == "lo-fp:".len() + 9,
            "{fp}"
        );
        assert_eq!(&fp[10..11], "-");
        assert_ne!(
            fp,
            fingerprint(
                [("a:skill/x", "blake3:1", false), items[1]],
                ["claude-code"]
            )
        );
        assert_ne!(
            fp,
            fingerprint([("a:skill/x", "blake3:9", true), items[1]], ["claude-code"])
        );
        assert_ne!(fp, fingerprint(items, ["claude-code", "pi"]));
        assert_ne!(fp, fingerprint(items[..1].iter().copied(), ["claude-code"]));
    }

    proptest! {
        #[test]
        fn fingerprint_ignores_order(mut ids in proptest::collection::vec("[a-z]{1,8}", 0..12), seed in any::<u64>()) {
            ids.sort();
            ids.dedup();
            let items: Vec<(String, String, bool)> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| (format!("s:skill/{id}"), format!("blake3:{i}"), i % 2 == 0))
                .collect();
            let mut shuffled = items.clone();
            let n = shuffled.len().max(1);
            shuffled.rotate_left((seed as usize) % n);
            shuffled.reverse();
            let fp = |v: &[(String, String, bool)]| {
                fingerprint(v.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), *c)), ["pi", "claude-code"])
            };
            prop_assert_eq!(fp(&items), fp(&shuffled));
        }

        #[test]
        fn any_payload_round_trips(
            company_config in proptest::option::of("[ -~]{0,40}"),
            toggles in proptest::collection::btree_map("[a-z:/-]{1,20}", any::<bool>(), 0..6),
            targets in proptest::collection::vec("[a-z-]{1,12}", 0..4),
        ) {
            let p = Payload { v: PAYLOAD_VERSION, company_config, toggles, targets, ..Payload::default() };
            prop_assert_eq!(decode(&encode(&p)).unwrap(), p);
        }
    }
}
