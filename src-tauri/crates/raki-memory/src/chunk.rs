use raki_domain::body::body_to_blocks;

const CHUNK_CAP: usize = 1600;
const MAX_CHUNKS_PER_NOTE: usize = 32;

/// A searchable chunk tied to the stable block ID it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Stable identifier of the source block (with split index appended by the chunker).
    pub block_id: String,
    /// Searchable text for embedding and retrieval.
    pub text: String,
}

/// Split text into chunks no longer than `CHUNK_CAP` bytes.
/// Prefer splitting at whitespace. If a single word exceeds the cap, keep it intact.
pub fn cap_split(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![];
    }
    if text.len() <= CHUNK_CAP {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + CHUNK_CAP).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let split_at = if end == text.len() {
            end
        } else {
            text[start..end]
                .rfind(' ')
                .map(|i| start + i)
                .unwrap_or(end)
        };
        chunks.push(text[start..split_at].to_string());
        start = split_at;
        if start < text.len() && text.as_bytes().get(start) == Some(&b' ') {
            start += 1;
        }
    }
    chunks
}

/// Chunk a note's body into searchable strings.
///
/// - Calls `body_to_blocks` to extract structural blocks.
/// - Optionally prefixes each block with its title (and heading, if present).
/// - Applies `cap_split` so no chunk exceeds the character cap.
/// - Truncates to `MAX_CHUNKS_PER_NOTE` with a warning.
/// - If no blocks are produced, returns a single chunk containing the title.
pub fn chunk_note(title: &str, body: &str, use_prefix: bool) -> Vec<Chunk> {
    let blocks = body_to_blocks(body);
    if blocks.is_empty() {
        if title.is_empty() {
            return vec![];
        }
        return vec![Chunk {
            block_id: "title".to_string(),
            text: title.to_string(),
        }];
    }

    let mut chunks: Vec<Chunk> = Vec::new();

    for block in &blocks {
        let block_id = block.block_id.clone().unwrap_or_else(|| "none".to_string());
        let prefixed = if use_prefix {
            match &block.heading {
                Some(heading) => format!("{} > {}: {}", title, heading, block.text),
                None => format!("{}: {}", title, block.text),
            }
        } else {
            block.text.clone()
        };

        let split = cap_split(&prefixed);
        for (i, text) in split.into_iter().enumerate() {
            chunks.push(Chunk {
                block_id: format!("{}:{}", block_id, i),
                text,
            });
        }

        if chunks.len() >= MAX_CHUNKS_PER_NOTE {
            break;
        }
    }

    if chunks.len() > MAX_CHUNKS_PER_NOTE {
        tracing::warn!(
            "Note {:?} produced {} chunks, truncating to {}",
            title,
            chunks.len(),
            MAX_CHUNKS_PER_NOTE
        );
        chunks.truncate(MAX_CHUNKS_PER_NOTE);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_note_bare_blocks() {
        let body = r#"{"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"first"}]},
            {"type":"paragraph","content":[{"type":"text","text":"second"}]}
        ]}"#;
        let chunks = chunk_note("My Note", body, false);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "first");
        assert_eq!(chunks[1].text, "second");
        assert_eq!(chunks[0].block_id, "none:0");
        assert_eq!(chunks[1].block_id, "none:0");
    }

    #[test]
    fn chunk_note_with_prefix() {
        let body = r#"{"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"hello world"}]}
        ]}"#;
        let chunks = chunk_note("My Note", body, true);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "My Note: hello world");
        assert!(chunks[0].block_id.starts_with("none:0"));
    }

    #[test]
    fn chunk_note_with_heading_prefix() {
        let body = r#"{"type":"doc","content":[
            {"type":"heading","content":[{"type":"text","text":"Intro"}]},
            {"type":"paragraph","content":[{"type":"text","text":"body text"}]}
        ]}"#;
        let chunks = chunk_note("My Note", body, true);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "My Note > Intro: body text");
    }

    #[test]
    fn chunk_note_empty_body_returns_title() {
        let chunks = chunk_note("My Note", "", false);
        assert_eq!(
            chunks,
            vec![Chunk {
                block_id: "title".to_string(),
                text: "My Note".to_string()
            }]
        );
    }

    #[test]
    fn chunk_note_zero_block_body_returns_title() {
        let body = r#"{"type":"doc","content":[{"type":"horizontalRule"}]}"#;
        let chunks = chunk_note("My Note", body, false);
        assert_eq!(
            chunks,
            vec![Chunk {
                block_id: "title".to_string(),
                text: "My Note".to_string()
            }]
        );
    }

    #[test]
    fn cap_split_does_not_silently_truncate() {
        let long = "word ".repeat(400);
        let chunks = cap_split(&long);
        let recovered = chunks.join(" ");
        assert_eq!(recovered, long); // exact match, no trimming
        for chunk in &chunks {
            assert!(
                chunk.len() <= CHUNK_CAP,
                "chunk too long: {} chars",
                chunk.len()
            );
        }
    }

    #[test]
    fn cap_split_preserves_multiple_spaces() {
        let text = "a  b   c";
        let chunks = cap_split(text);
        assert_eq!(chunks, vec!["a  b   c"]);
    }

    #[test]
    fn cap_split_single_word_exceeds_cap() {
        let word = "x".repeat(2000);
        let chunks = cap_split(&word);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "x".repeat(CHUNK_CAP));
        assert_eq!(chunks[1], "x".repeat(2000 - CHUNK_CAP));
    }

    #[test]
    fn cap_split_respects_multibyte_char_boundaries() {
        // '大' is 3 bytes in UTF-8. 534 repetitions = 1602 bytes, so byte 1600
        // falls inside a character and would panic if we sliced blindly.
        let text = "大".repeat(534);
        let chunks = cap_split(&text);
        assert!(!chunks.is_empty());
        let recovered = chunks.join("");
        assert_eq!(recovered, text);
        for chunk in &chunks {
            assert!(
                chunk.len() <= CHUNK_CAP,
                "chunk too long: {} bytes",
                chunk.len()
            );
        }
    }

    #[test]
    fn cap_split_empty_string() {
        let chunks = cap_split("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_note_long_prefixed_block_splits() {
        let long_text = "word ".repeat(400);
        let body = format!(
            r#"{{"type":"doc","content":[{{"type":"paragraph","content":[{{"type":"text","text":"{}"}}]}}]}}"#,
            long_text
        );
        let chunks = chunk_note("My Note", &body, true);
        assert!(
            chunks.len() > 1,
            "expected multiple chunks, got {}",
            chunks.len()
        );
        let recovered = chunks
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            recovered.contains(&long_text),
            "all chunks should be present in recovered text"
        );
        for chunk in &chunks {
            assert!(
                chunk.text.len() <= CHUNK_CAP,
                "chunk too long: {} chars",
                chunk.text.len()
            );
        }
    }

    #[test]
    fn chunk_note_respects_max_chunks() {
        let mut paragraphs = Vec::new();
        for i in 0..40 {
            paragraphs.push(format!(
                r#"{{"type":"paragraph","content":[{{"type":"text","text":"{}"}}]}}"#,
                i
            ));
        }
        let body = format!(r#"{{"type":"doc","content":[{}]}}"#, paragraphs.join(","));
        let chunks = chunk_note("My Note", &body, false);
        assert_eq!(chunks.len(), MAX_CHUNKS_PER_NOTE);
    }
}
