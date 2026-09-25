//! Content hashing for items (lockfile `content_hash`, fingerprints).

/// Prefix of every content hash string.
pub const HASH_PREFIX: &str = "blake3:";

/// Hashes a file tree given as `(relative path, contents)` pairs.
///
/// Paths use `/` separators. The result is independent of input order and
/// distinguishes file boundaries, so moving bytes between files changes it.
pub fn tree_hash<'a, I>(files: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a [u8])>,
{
    let mut files: Vec<(&str, &[u8])> = files.into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(b.0));
    let mut h = blake3::Hasher::new();
    h.update(b"loadout-tree-v1\0");
    for (path, contents) in files {
        h.update(&(path.len() as u64).to_le_bytes());
        h.update(path.as_bytes());
        h.update(&(contents.len() as u64).to_le_bytes());
        h.update(contents);
    }
    format!("{HASH_PREFIX}{}", h.finalize().to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_independent() {
        let a = tree_hash([("a", &b"1"[..]), ("b/c", &b"2"[..])]);
        let b = tree_hash([("b/c", &b"2"[..]), ("a", &b"1"[..])]);
        assert_eq!(a, b);
        assert!(a.starts_with("blake3:"));
        assert_eq!(a.len(), "blake3:".len() + 64);
    }

    #[test]
    fn sensitive_to_paths_contents_and_boundaries() {
        let base = tree_hash([("a", &b"12"[..])]);
        assert_ne!(base, tree_hash([("a", &b"13"[..])]));
        assert_ne!(base, tree_hash([("b", &b"12"[..])]));
        assert_ne!(base, tree_hash([("a", &b"1"[..]), ("a2", &b""[..])]));
        assert_ne!(tree_hash([]), tree_hash([("", &b""[..])]));
    }
}
