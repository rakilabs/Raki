//! Conversion between the canonical ProseMirror-JSON note body (ADR-0004) and the plain
//! text the editor works in. These are the single, total definitions shared by storage
//! indexing, QA context assembly, and the command layer, so the format rule cannot drift.

use serde_json::{json, Value};

/// Flatten a canonical ProseMirror `doc` to plain text: each top-level block's text joined
/// with `\n` between blocks; text nodes within a block concatenated directly (their own
/// spacing is preserved). Total and defensive — never panics:
/// - the empty marker `"{}"`, an empty `doc`, or a contentless `doc` → `""` (review C1)
/// - structurally-odd but valid JSON → best-effort text, unknown nodes skipped (review M2)
/// - genuinely non-JSON input → returned verbatim (a legacy/plain body stays editable)
pub fn body_to_text(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return body.to_string(); // not JSON at all → treat as raw text
    };
    if value.get("type").and_then(Value::as_str) != Some("doc") {
        // legacy empty marker "{}" → blank; any other non-doc JSON → raw text
        if body == "{}" {
            return String::new();
        }
        return body.to_string();
    }
    let mut blocks: Vec<String> = Vec::new();
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        for block in content {
            let mut text = String::new();
            collect_text(block, &mut text);
            blocks.push(text);
        }
    }
    blocks.join("\n")
}

/// Depth-first collect every `text` node's string (no separators — block separation is the
/// caller's job). Skips any node without a text payload; recurses through `content`.
fn collect_text(node: &Value, out: &mut String) {
    if let Some(t) = node.get("text").and_then(Value::as_str) {
        out.push_str(t);
    }
    if let Some(content) = node.get("content").and_then(Value::as_array) {
        for child in content {
            collect_text(child, out);
        }
    }
}

/// A structural block extracted from a ProseMirror document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The most recent heading text that precedes this block in the document.
    pub heading: Option<String>,
    /// The plain text content of the block.
    pub text: String,
}

/// Parse a canonical ProseMirror `doc` into structural blocks.
///
/// - `paragraph` → one block
/// - `bulletList` / `orderedList` → one block (list items joined with `\n`)
/// - `codeBlock` → one block
/// - `heading` → updates the running heading context for subsequent blocks (not emitted)
/// - other nodes → skipped
/// - invalid JSON or non-doc → empty vec
pub fn body_to_blocks(body: &str) -> Vec<Block> {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Vec::new();
    };
    if value.get("type").and_then(Value::as_str) != Some("doc") {
        return Vec::new();
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut current_heading: Option<String> = None;
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        for node in content {
            let node_type = node.get("type").and_then(Value::as_str);
            match node_type {
                Some("heading") => {
                    let mut text = String::new();
                    collect_text(node, &mut text);
                    current_heading = Some(text);
                }
                Some("paragraph") | Some("codeBlock") => {
                    let mut text = String::new();
                    collect_text(node, &mut text);
                    blocks.push(Block {
                        heading: current_heading.clone(),
                        text,
                    });
                }
                Some("bulletList") | Some("orderedList") => {
                    let mut items: Vec<String> = Vec::new();
                    if let Some(list_content) = node.get("content").and_then(Value::as_array) {
                        for item in list_content {
                            let mut item_text = String::new();
                            collect_text(item, &mut item_text);
                            items.push(item_text);
                        }
                    }
                    blocks.push(Block {
                        heading: current_heading.clone(),
                        text: items.join("\n"),
                    });
                }
                _ => {}
            }
        }
    }
    blocks
}

/// Wrap plain editor text into a canonical ProseMirror `doc`: one `paragraph` per line,
/// each holding a single `text` node (empty lines → empty paragraphs). Empty input → an
/// empty `doc`. Inverse of `body_to_text` for line-separated plain text.
pub fn text_to_body(text: &str) -> String {
    let content: Vec<Value> = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n')
            .map(|line| {
                if line.is_empty() {
                    json!({ "type": "paragraph" })
                } else {
                    json!({ "type": "paragraph", "content": [{ "type": "text", "text": line }] })
                }
            })
            .collect()
    };
    json!({ "type": "doc", "content": content }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_a_doc_blocks_with_newlines_text_nodes_directly() {
        let doc = r#"{"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"Pay cash"},{"type":"text","text":" at the ryokan."}]},
            {"type":"paragraph","content":[{"type":"text","text":"Checkout is 10am."}]}
        ]}"#;
        assert_eq!(
            body_to_text(doc),
            "Pay cash at the ryokan.\nCheckout is 10am."
        );
    }

    #[test]
    fn empty_marker_and_empty_doc_are_blank_not_raw() {
        // review C1: legacy "{}" must NOT surface as literal text.
        assert_eq!(body_to_text("{}"), "");
        assert_eq!(body_to_text(r#"{"type":"doc","content":[]}"#), "");
    }

    #[test]
    fn doc_without_content_is_blank_and_nested_nodes_are_walked_without_panic() {
        // review M2: total/defensive on odd-but-valid doc JSON.
        assert_eq!(body_to_text(r#"{"type":"doc"}"#), "");
        let nested = r#"{"type":"doc","content":[
            {"type":"bulletList","content":[
                {"type":"listItem","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}]}]}"#;
        assert_eq!(body_to_text(nested), "hi");
    }

    #[test]
    fn non_json_falls_back_to_raw() {
        assert_eq!(body_to_text("just plain text"), "just plain text");
    }

    #[test]
    fn text_to_body_round_trips_line_separated_text() {
        for t in ["", "one line", "a\nb", "a\n\nb"] {
            assert_eq!(body_to_text(&text_to_body(t)), t, "round-trip {t:?}");
        }
    }

    #[test]
    fn text_to_body_emits_a_canonical_doc() {
        assert_eq!(text_to_body(""), r#"{"content":[],"type":"doc"}"#);
    }

    #[test]
    fn body_to_blocks_extracts_paragraphs() {
        let doc = r#"{"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"first"}]},
            {"type":"paragraph","content":[{"type":"text","text":"second"}]}
        ]}"#;
        let blocks = body_to_blocks(doc);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].heading, None);
        assert_eq!(blocks[0].text, "first");
        assert_eq!(blocks[1].heading, None);
        assert_eq!(blocks[1].text, "second");
    }

    #[test]
    fn body_to_blocks_joins_list_items() {
        let doc = r#"{"type":"doc","content":[
            {"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"milk"}]}]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"eggs"}]}]}
            ]}
        ]}"#;
        let blocks = body_to_blocks(doc);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "milk\neggs");
    }

    #[test]
    fn body_to_blocks_tracks_headings() {
        let doc = r#"{"type":"doc","content":[
            {"type":"heading","content":[{"type":"text","text":"Intro"}]},
            {"type":"paragraph","content":[{"type":"text","text":"body"}]}
        ]}"#;
        let blocks = body_to_blocks(doc);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].heading, Some("Intro".to_string()));
        assert_eq!(blocks[0].text, "body");
    }

    #[test]
    fn body_to_blocks_returns_empty_for_invalid_json() {
        let blocks = body_to_blocks("not json");
        assert!(blocks.is_empty());
    }
}
