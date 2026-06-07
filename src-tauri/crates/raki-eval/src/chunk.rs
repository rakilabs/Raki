//! Chunking strategies for the eval. Pure: turns a note's (title, body) into the chunk *texts*
//! to embed. The eval composes chunk *ids* (note-uuid for WholeNote, `uuid#i` for Blocks).

use crate::markdown::to_blocks;

/// Granularity: the whole note as one chunk (today's behavior) vs structural blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkStrategy {
    WholeNote,
    Blocks,
}

/// What context to prepend to a block chunk (a measured arm; D6). Inert for WholeNote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixMode {
    Bare,
    Title,
    TitleHeading,
}

/// How chunk hits roll up to a note ranking (a measured arm; D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rollup {
    MinRank,
    ScoreMax,
}

/// Approximate per-chunk character cap (~ conservative <512 bge tokens at ~3.1 chars/token).
/// A correctness floor against silent embedding truncation (D2), NOT quality tuning.
pub const CHUNK_CHAR_CAP: usize = 1600;

/// Split `text` into pieces no longer than `cap` chars, breaking on a space near the cap when
/// possible (never silently truncating). Returns at least one piece.
pub fn cap_split(text: &str, cap: usize) -> Vec<String> {
    if text.len() <= cap {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut rest = text;
    while rest.len() > cap {
        // Find a char boundary <= cap; prefer the last space before it for a clean break.
        let mut end = cap;
        while end > 0 && !rest.is_char_boundary(end) {
            end -= 1;
        }
        let split = rest[..end].rfind(' ').map(|s| s + 1).unwrap_or(end).max(1);
        out.push(rest[..split].trim().to_string());
        rest = &rest[split..];
    }
    if !rest.trim().is_empty() {
        out.push(rest.trim().to_string());
    }
    out
}

/// Produce the chunk texts to embed. `WholeNote` returns exactly `["{title}\n\n{body}"]` (byte-
/// identical to the legacy path). `Blocks` splits the body, applies the prefix arm, and token-caps.
pub fn chunk(title: &str, body: &str, strategy: ChunkStrategy, prefix: PrefixMode) -> Vec<String> {
    match strategy {
        ChunkStrategy::WholeNote => vec![format!("{title}\n\n{body}")],
        ChunkStrategy::Blocks => {
            let mut out = Vec::new();
            for b in to_blocks(body) {
                let prefixed = match prefix {
                    PrefixMode::Bare => b.text.clone(),
                    PrefixMode::Title => format!("{title} — {}", b.text),
                    PrefixMode::TitleHeading => match &b.heading {
                        Some(h) => format!("{title} — {h} — {}", b.text),
                        None => format!("{title} — {}", b.text),
                    },
                };
                out.extend(cap_split(&prefixed, CHUNK_CHAR_CAP));
            }
            if out.is_empty() {
                // A body with no parseable content blocks still needs one chunk.
                out.push(format!("{title}\n\n{body}"));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_note_is_one_chunk_identical_to_legacy_doc() {
        let out = chunk(
            "Title",
            "Body text.",
            ChunkStrategy::WholeNote,
            PrefixMode::TitleHeading,
        );
        assert_eq!(out, vec!["Title\n\nBody text.".to_string()]);
    }

    #[test]
    fn blocks_split_and_apply_prefix_arms() {
        let body = "## Sec\nFirst para fact.\n\nSecond para.\n";
        let bare = chunk("T", body, ChunkStrategy::Blocks, PrefixMode::Bare);
        assert_eq!(bare.len(), 2);
        assert!(bare[0].starts_with("First para"));
        let titled = chunk("T", body, ChunkStrategy::Blocks, PrefixMode::Title);
        assert!(titled[0].starts_with("T — First para"));
        let th = chunk("T", body, ChunkStrategy::Blocks, PrefixMode::TitleHeading);
        assert!(th[0].starts_with("T — Sec — First para"));
    }

    #[test]
    fn cap_split_never_truncates_a_long_block() {
        let long = "word ".repeat(1000); // ~5000 chars
        let pieces = cap_split(&long, CHUNK_CHAR_CAP);
        assert!(pieces.len() >= 3, "split into multiple pieces");
        assert!(pieces.iter().all(|p| p.len() <= CHUNK_CHAR_CAP));
        // every word is preserved across the pieces (no silent loss).
        let total_words: usize = pieces.iter().map(|p| p.split_whitespace().count()).sum();
        assert_eq!(total_words, 1000);
    }
}
