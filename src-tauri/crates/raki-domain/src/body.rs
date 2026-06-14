//! Conversion between the canonical ProseMirror-JSON note body (ADR-0004) and the plain
//! text the editor works in. These are the single, total definitions shared by storage
//! indexing, QA context assembly, and the command layer, so the format rule cannot drift.

use serde_json::{json, Value};

use crate::DomainError;

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
    /// Stable identifier assigned to this top-level block, if present.
    pub block_id: Option<String>,
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
            let block_id = node
                .get("attrs")
                .and_then(|a| a.get("blockId"))
                .and_then(Value::as_str)
                .map(String::from);
            let node_type = node.get("type").and_then(Value::as_str);
            match node_type {
                Some("heading") => {
                    let mut text = String::new();
                    collect_text(node, &mut text);
                    current_heading = if text.is_empty() { None } else { Some(text) };
                }
                Some("paragraph") | Some("codeBlock") => {
                    let mut text = String::new();
                    collect_text(node, &mut text);
                    if !text.is_empty() {
                        blocks.push(Block {
                            heading: current_heading.clone(),
                            text,
                            block_id,
                        });
                    }
                }
                Some("bulletList") | Some("orderedList") => {
                    let mut items: Vec<String> = Vec::new();
                    if let Some(list_content) = node.get("content").and_then(Value::as_array) {
                        for item in list_content {
                            let mut item_text = String::new();
                            collect_text(item, &mut item_text);
                            if !item_text.is_empty() {
                                items.push(item_text);
                            }
                        }
                    }
                    let text = items.join("\n");
                    if !text.is_empty() {
                        blocks.push(Block {
                            heading: current_heading.clone(),
                            text,
                            block_id,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    blocks
}

use uuid::Uuid;

/// Normalize a ProseMirror-JSON body: ensure it is a valid `doc` and that every top-level
/// block has a stable `blockId`. Existing block IDs are preserved; missing ones are assigned.
pub fn normalize_body(body: &str) -> Result<String, DomainError> {
    let mut value: Value = serde_json::from_str(body)
        .map_err(|e| DomainError::Invalid(format!("invalid body json: {e}")))?;
    if value.get("type").and_then(Value::as_str) != Some("doc") {
        return Err(DomainError::Invalid(
            "body must be a ProseMirror doc".into(),
        ));
    }
    assign_block_ids(&mut value);
    Ok(value.to_string())
}

/// Assign a UUIDv7 block ID to every top-level block node that does not already have one.
pub fn assign_block_ids(doc: &mut Value) {
    let Some(content) = doc.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    for node in content.iter_mut() {
        if !is_top_level_block(node) {
            continue;
        }
        let attrs = node
            .as_object_mut()
            .unwrap()
            .entry("attrs")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap();
        if !attrs.contains_key("blockId") {
            attrs.insert("blockId".to_string(), json!(new_block_id()));
        }
    }
}

fn is_top_level_block(node: &Value) -> bool {
    matches!(
        node.get("type").and_then(Value::as_str),
        Some("paragraph")
            | Some("heading")
            | Some("bulletList")
            | Some("orderedList")
            | Some("codeBlock")
    )
}

fn new_block_id() -> String {
    Uuid::now_v7().to_string()
}

/// Wrap plain editor text into a canonical ProseMirror `doc`: one `paragraph` per line,
/// each holding a single `text` node (empty lines → empty paragraphs). Empty input → an
/// empty `doc`. Inverse of `body_to_text` for line-separated plain text.
pub fn text_to_body(text: &str) -> String {
    let mut doc: Value = if text.is_empty() {
        json!({ "type": "doc", "content": [] })
    } else {
        let content: Vec<Value> = text
            .split('\n')
            .map(|line| {
                if line.is_empty() {
                    json!({ "type": "paragraph" })
                } else {
                    json!({
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": line }]
                    })
                }
            })
            .collect();
        json!({ "type": "doc", "content": content })
    };
    assign_block_ids(&mut doc);
    doc.to_string()
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
    fn body_to_blocks_extracts_code_blocks() {
        let doc = r#"{"type":"doc","content":[{"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"fn main() {}"}]}]}"#;
        let blocks = body_to_blocks(doc);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].heading, None);
        assert_eq!(blocks[0].text, "fn main() {}");
    }

    #[test]
    fn body_to_blocks_returns_empty_for_invalid_json() {
        let blocks = body_to_blocks("not json");
        assert!(blocks.is_empty());
    }

    #[test]
    fn body_to_blocks_empty_doc_returns_empty_vec() {
        let blocks = body_to_blocks("{}");
        assert!(blocks.is_empty());
    }

    #[test]
    fn body_to_blocks_skips_unknown_nodes() {
        let doc = r#"{"type":"doc","content":[{"type":"horizontalRule"}]}"#;
        let blocks = body_to_blocks(doc);
        assert!(blocks.is_empty());
    }

    #[test]
    fn normalize_body_assigns_ids_to_blocks_missing_them() {
        let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}"#;
        let out = normalize_body(body).unwrap();
        assert!(out.contains("blockId"));
    }

    #[test]
    fn normalize_body_preserves_existing_block_ids() {
        let body = r#"{"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"existing-id"},"content":[{"type":"text","text":"hi"}]}]}"#;
        let out = normalize_body(body).unwrap();
        assert!(out.contains("\"blockId\":\"existing-id\""));
    }

    #[test]
    fn normalize_body_rejects_non_doc_json() {
        let body = r#"{"type":"not-a-doc"}"#;
        assert!(normalize_body(body).is_err());
    }

    #[test]
    fn body_to_blocks_extracts_block_id() {
        let doc = r#"{"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"bid-1"},"content":[{"type":"text","text":"first"}]}]}"#;
        let blocks = body_to_blocks(doc);
        assert_eq!(blocks[0].block_id, Some("bid-1".to_string()));
    }

    #[test]
    fn text_to_body_assigns_block_ids() {
        let body = text_to_body("line one\nline two");
        let blocks = body_to_blocks(&body);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].block_id.is_some());
        assert!(blocks[1].block_id.is_some());
        assert_ne!(blocks[0].block_id, blocks[1].block_id);
    }
}
