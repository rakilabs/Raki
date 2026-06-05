//! Stable content hash for embedding-staleness detection. NOT a security hash —
//! a fast, deterministic change-detector. A wrong hash silently breaks the cache
//! (never re-embed → stale vectors; always re-embed → wasted compute), so the
//! definition is pinned here, not left loose.

use unicode_normalization::UnicodeNormalization;

/// FNV-1a 64-bit over NFC-normalized, whitespace-collapsed `title` + `body`.
/// Volatile fields (timestamps, version, id, deleted_at) are deliberately excluded.
pub fn content_hash(title: &str, body: &str) -> String {
    let normalized = format!("{}\u{0}{}", normalize(title), normalize(body));
    let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
    for b in normalized.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3); // FNV prime
    }
    format!("{h:016x}")
}

/// NFC-normalize, collapse internal whitespace runs to a single space, and trim.
fn normalize(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    nfc.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_whitespace_insensitive() {
        assert_eq!(
            content_hash("Hello", "World"),
            content_hash("Hello", "World")
        );
        assert_eq!(
            content_hash("  Hello ", "World"),
            content_hash("Hello", "World")
        );
        assert_eq!(
            content_hash("Hello   World", ""),
            content_hash("Hello World", "")
        );
    }

    #[test]
    fn distinguishes_content_and_field_boundary() {
        // different content → different hash
        assert_ne!(content_hash("a", "b"), content_hash("a", "c"));
        // the field separator prevents "ab"+"" colliding with "a"+"b"
        assert_ne!(content_hash("ab", ""), content_hash("a", "b"));
    }
}
