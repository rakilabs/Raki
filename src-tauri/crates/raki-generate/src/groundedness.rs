//! The deterministic groundedness verdict. No model call: parse-or-fail-closed, then classify
//! against the context's source ids. See spec D4.

use std::collections::HashSet;

use raki_domain::SourceId;
use serde::Deserialize;

/// The answer's relationship to the retrieved context. Richer than a bool so the UI and a future
/// `qa-report` can distinguish the failure modes (spec D4 / Slice 1 line 185).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerState {
    NothingMatched,
    NotAnswerable,
    ParseFailed,
    Ungrounded,
    Grounded,
}

impl AnswerState {
    pub fn name(&self) -> &'static str {
        match self {
            AnswerState::NothingMatched => "nothing_matched",
            AnswerState::NotAnswerable => "not_answerable",
            AnswerState::ParseFailed => "parse_failed",
            AnswerState::Ungrounded => "ungrounded",
            AnswerState::Grounded => "grounded",
        }
    }
    /// The persisted bit (spec D5): only `Grounded` is true.
    pub fn is_grounded(&self) -> bool {
        matches!(self, AnswerState::Grounded)
    }
}

#[derive(Deserialize)]
struct ModelReply {
    #[serde(default)]
    answer: String,
    // `Option` so an explicit `null` (not just a missing field) is tolerated, not a parse error
    // (review #5). `unwrap_or_default()` below maps both null and missing to empty/false.
    #[serde(default)]
    cited_source_ids: Option<Vec<String>>,
    #[serde(default)]
    insufficient_context: Option<bool>,
}

/// Candidate JSON substrings in priority order: a fenced ```json … ``` block first (the model put
/// the answer there deliberately), then every balanced top-level `{…}` object. The caller try-parses
/// each and uses the first that fits `ModelReply` — so prose containing decoy braces before the real
/// object no longer forces `ParseFailed` (review #1). String contents are skipped so a `{` or `}`
/// inside a JSON string value can't miscount depth.
fn candidate_blocks(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(start) = raw.find("```") {
        let after = &raw[start + 3..];
        let trimmed = after.trim_start();
        let after = if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("json") {
            &trimmed[4..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            out.push(after[..end].trim());
        }
    }
    out.extend(balanced_objects(raw));
    out
}

/// Every top-level balanced `{…}` object in `raw`, in order, string/escape aware.
fn balanced_objects(raw: &str) -> Vec<&str> {
    let b = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'{' {
            i += 1;
            continue;
        }
        let (mut depth, mut in_str, mut esc) = (0usize, false, false);
        let mut j = i;
        while j < b.len() {
            let c = b[j];
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            out.push(&raw[i..=j]);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        i = j + 1;
    }
    out
}

fn first_parseable(raw: &str) -> Option<ModelReply> {
    candidate_blocks(raw)
        .into_iter()
        .find_map(|c| serde_json::from_str::<ModelReply>(c).ok())
}

/// Classify a raw model reply against the context ids. Returns (state, answer_text, cited).
pub fn evaluate(raw: &str, context_ids: &HashSet<String>) -> (AnswerState, String, Vec<SourceId>) {
    let Some(reply) = first_parseable(raw) else {
        return (AnswerState::ParseFailed, raw.to_string(), vec![]);
    };
    if reply.insufficient_context.unwrap_or(false) {
        return (AnswerState::NotAnswerable, reply.answer, vec![]);
    }
    // Dedup citations (m3), preserving order.
    let mut seen = HashSet::new();
    let cites: Vec<String> = reply
        .cited_source_ids
        .unwrap_or_default()
        .into_iter()
        .filter(|c| seen.insert(c.clone()))
        .collect();
    if cites.is_empty() {
        return (AnswerState::Ungrounded, reply.answer, vec![]); // review #2/M10: no provenance
    }
    if cites.iter().any(|c| !context_ids.contains(c)) {
        let ids = cites.into_iter().map(SourceId).collect();
        return (AnswerState::Ungrounded, reply.answer, ids); // fabricated citation
    }
    let ids = cites.into_iter().map(SourceId).collect();
    (AnswerState::Grounded, reply.answer, ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn grounded_when_all_cites_present() {
        let raw = r#"{"answer":"yes","cited_source_ids":["n1"],"insufficient_context":false}"#;
        let (s, text, cited) = evaluate(raw, &ctx(&["n1", "n2"]));
        assert_eq!(s, AnswerState::Grounded);
        assert_eq!(text, "yes");
        assert_eq!(cited, vec![SourceId("n1".into())]);
        assert!(s.is_grounded());
    }

    #[test]
    fn tolerates_markdown_fence() {
        let raw = "```json\n{\"answer\":\"ok\",\"cited_source_ids\":[\"n1\"]}\n```";
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Grounded);
    }

    #[test]
    fn not_answerable_on_sentinel() {
        let raw = r#"{"answer":"I don't know","insufficient_context":true}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::NotAnswerable);
    }

    #[test]
    fn ungrounded_when_zero_citations() {
        let raw = r#"{"answer":"sky is blue","cited_source_ids":[]}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Ungrounded);
    }

    #[test]
    fn ungrounded_when_citation_not_in_context() {
        let raw = r#"{"answer":"x","cited_source_ids":["n9"]}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Ungrounded);
    }

    #[test]
    fn parse_failed_on_non_json() {
        assert_eq!(
            evaluate("I cannot comply.", &ctx(&["n1"])).0,
            AnswerState::ParseFailed
        );
    }

    #[test]
    fn skips_decoy_braces_before_the_real_json() {
        // review #1: prose with a non-JSON brace pair, then the real fenced object.
        let raw = "Here is the answer: {not available}\n```json\n{\"answer\":\"yes\",\"cited_source_ids\":[\"n1\"]}\n```";
        let (s, text, _) = evaluate(raw, &ctx(&["n1"]));
        assert_eq!(s, AnswerState::Grounded);
        assert_eq!(text, "yes");
    }

    #[test]
    fn null_citations_are_ungrounded_not_parse_failed() {
        // review #5: explicit null array → 0 citations → Ungrounded, not ParseFailed.
        let raw = r#"{"answer":"x","cited_source_ids":null,"insufficient_context":null}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Ungrounded);
    }
}
