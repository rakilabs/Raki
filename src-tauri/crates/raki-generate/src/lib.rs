//! Grounded QA orchestration: retrieve → assemble → gate → answer → verify. Composes the leaf
//! crates (the dependency rule forbids a leaf from doing this — see spec "Crate placement").

mod groundedness;

pub use groundedness::AnswerState;

use raki_ai::GatedLlmProvider;
use raki_domain::{
    DomainError, EgressError, EmbeddingProvider, KeywordIndex, NoteRepository, SourceId,
    VectorIndex,
};

/// Everything `answer_question` needs, injected so the flow is fake-testable.
pub struct GenerateDeps<'a> {
    pub keyword: &'a dyn KeywordIndex,
    pub vectors: &'a dyn VectorIndex,
    pub embedder: &'a dyn EmbeddingProvider, // assumed LOCAL (spec M4)
    pub notes: &'a dyn NoteRepository,
    pub gate: &'a GatedLlmProvider,
    pub provider: &'a str,
    pub model: &'a str,
    pub budget: usize,
    pub k: usize,
}

/// The result of a QA request.
pub struct Answer {
    pub state: AnswerState,
    pub text: String,
    pub cited_ids: Vec<SourceId>,
    pub egress_log_id: Option<raki_domain::EgressLogId>,
}

/// Non-egress vs egress failures stay distinguishable (spec C2).
#[derive(Debug)]
pub enum GenerateError {
    Egress(EgressError),
    Domain(DomainError),
}

use raki_domain::{CompletionRequest, NoteId};
use raki_memory::{assemble_context, AssembledContext, Candidate};
use raki_retrieval::hybrid_search;

use groundedness::evaluate;

/// System prompt: bind the model to the numbered context and the JSON reply contract (spec D4).
fn build_system_prompt(ctx: &AssembledContext) -> String {
    let mut s = String::from(
        "You answer ONLY from the notes below. Reply with a single JSON object and nothing else: \
         {\"answer\": string, \"cited_source_ids\": [string], \"insufficient_context\": bool}. \
         Cite the source_id of every note you used. If the notes do not contain the answer, set \
         insufficient_context to true.\n\nNOTES:\n",
    );
    for item in &ctx.items {
        s.push_str(&format!("[{}] {}\n", item.source_id, item.text));
    }
    s
}

/// Best-effort plain text from a note body. If `body` is ProseMirror JSON, extract text
/// nodes recursively; otherwise pass through unchanged (plain text or legacy format).
fn note_body_to_text(body: &str) -> String {
    fn extract_text(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(text);
                }
                if let Some(content) = map.get("content").and_then(|v| v.as_array()) {
                    for child in content {
                        extract_text(child, out);
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for child in arr {
                    extract_text(child, out);
                }
            }
            _ => {}
        }
    }

    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if v.get("type").and_then(|t| t.as_str()) == Some("doc") => {
            let mut out = String::new();
            extract_text(&v, &mut out);
            out
        }
        _ => body.to_string(),
    }
}

pub async fn answer_question(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Answer, GenerateError> {
    let ids = hybrid_search(deps.keyword, deps.vectors, deps.embedder, query, deps.k)
        .await
        .map_err(GenerateError::Domain)?;

    // Resolve ids → notes. hybrid_search already ranked best-first; keep that order via descending
    // synthetic scores (assemble_context sorts by score). Missing notes are simply skipped.
    let mut candidates = Vec::new();
    for (rank, id) in ids.iter().enumerate() {
        let nid = match NoteId::parse(id) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("skipping malformed source id {id}: {e}");
                continue;
            }
        };
        if let Some(note) = deps.notes.get(&nid).await.map_err(GenerateError::Domain)? {
            candidates.push(Candidate {
                source_id: id.clone(),
                text: format!("{}\n{}", note.title, note_body_to_text(&note.body)),
                score: (ids.len() - rank) as f64,
            });
        }
    }

    if candidates.is_empty() {
        return Ok(Answer {
            state: AnswerState::NothingMatched,
            text: "No relevant notes found.".into(),
            cited_ids: vec![],
            egress_log_id: None,
        });
    }

    let ctx = assemble_context(&candidates, deps.budget, deps.provider, deps.model);
    let req = CompletionRequest {
        system: Some(build_system_prompt(&ctx)),
        prompt: query.to_string(),
        // `None` → the adapter applies its own `DEFAULT_MAX_TOKENS` (review #7: single source of truth).
        max_tokens: None,
    };

    let (completion, log_id) = deps
        .gate
        .complete_gated(&ctx.egress, req)
        .await
        .map_err(GenerateError::Egress)?;

    let context_ids: std::collections::HashSet<String> =
        ctx.egress.source_ids.iter().map(|s| s.0.clone()).collect();
    let (state, text, cited_ids) = evaluate(&completion.text, &context_ids);

    deps.gate
        .set_grounded(&log_id, state.is_grounded())
        .await
        .map_err(GenerateError::Domain)?;

    Ok(Answer {
        state,
        text,
        cited_ids,
        egress_log_id: Some(log_id),
    })
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use raki_ai::testing::FakeLlmProvider;
    use raki_domain::testing::FixedClock;
    use raki_domain::{
        EgressLog, EgressLogId, EgressRecord, EgressSettings, Embedding, KeywordHit, Mode, Note,
        NoteId, VectorHit,
    };

    // --- fakes (impl domain ports) ---
    struct OneVector(String); // returns a single source id from the vector index
    #[async_trait]
    impl VectorIndex for OneVector {
        async fn upsert(&self, _: &str, _: &Embedding) -> Result<(), DomainError> {
            Ok(())
        }
        async fn query(&self, _: &Embedding, _: usize) -> Result<Vec<VectorHit>, DomainError> {
            Ok(vec![VectorHit {
                source_id: self.0.clone(),
                distance: 0.1,
            }])
        }
    }
    struct NoKeyword;
    #[async_trait]
    impl KeywordIndex for NoKeyword {
        async fn query(&self, _: &str, _: usize) -> Result<Vec<KeywordHit>, DomainError> {
            Ok(vec![])
        }
    }
    struct FakeEmbed;
    #[async_trait]
    impl EmbeddingProvider for FakeEmbed {
        fn dimension(&self) -> usize {
            1
        }
        fn locality(&self) -> raki_domain::Locality {
            raki_domain::Locality::Local
        }
        fn model_id(&self) -> String {
            "fake".into()
        }
        async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
            Ok(inputs.iter().map(|_| Embedding(vec![0.0])).collect())
        }
    }
    struct OneNote(NoteId);
    #[async_trait]
    impl NoteRepository for OneNote {
        async fn upsert(&self, _: &Note) -> Result<(), DomainError> {
            Ok(())
        }
        async fn get(&self, id: &NoteId) -> Result<Option<Note>, DomainError> {
            Ok((*id == self.0)
                .then(|| Note::new("Trip".into(), "Pay cash at the ryokan.".into(), 0)))
        }
        async fn list(&self) -> Result<Vec<Note>, DomainError> {
            Ok(vec![])
        }
        async fn soft_delete(&self, _: &NoteId, _: i64) -> Result<(), DomainError> {
            Ok(())
        }
    }
    struct EmptyRepo;
    #[async_trait]
    impl NoteRepository for EmptyRepo {
        async fn upsert(&self, _: &Note) -> Result<(), DomainError> {
            Ok(())
        }
        async fn get(&self, _: &NoteId) -> Result<Option<Note>, DomainError> {
            Ok(None)
        }
        async fn list(&self) -> Result<Vec<Note>, DomainError> {
            Ok(vec![])
        }
        async fn soft_delete(&self, _: &NoteId, _: i64) -> Result<(), DomainError> {
            Ok(())
        }
    }
    #[derive(Default)]
    struct SpyLog {
        grounded: Mutex<Vec<(EgressLogId, bool)>>,
    }
    #[async_trait]
    impl EgressLog for SpyLog {
        async fn record(&self, _: &EgressRecord) -> Result<(), DomainError> {
            Ok(())
        }
        async fn set_grounded(&self, id: &EgressLogId, g: bool) -> Result<(), DomainError> {
            self.grounded.lock().unwrap().push((*id, g));
            Ok(())
        }
    }
    struct CloudSettings;
    #[async_trait]
    impl EgressSettings for CloudSettings {
        async fn mode(&self) -> Result<Mode, DomainError> {
            Ok(Mode::CloudAllowed)
        }
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(HashSet::from(["kimi".to_string()]))
        }
        async fn set_mode(&self, _: Mode) -> Result<(), DomainError> {
            Ok(())
        }
        async fn grant(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn revoke(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    fn gate(inner: Arc<dyn raki_domain::LlmProvider>, log: Arc<SpyLog>) -> GatedLlmProvider {
        GatedLlmProvider::new(
            inner,
            Arc::new(CloudSettings),
            log,
            Arc::new(FixedClock(1000)),
        )
    }

    #[tokio::test]
    async fn grounded_answer_sets_grounded_true() {
        let nid = NoteId::new();
        let reply = r#"{"answer":"Pay cash.","cited_source_ids":["IDPLACEHOLDER"],"insufficient_context":false}"#
            .replace("IDPLACEHOLDER", &nid.to_string());
        let fake = Arc::new(FakeLlmProvider::ok(&reply));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log.clone());
        let deps = GenerateDeps {
            keyword: &NoKeyword,
            vectors: &OneVector(nid.to_string()),
            embedder: &FakeEmbed,
            notes: &OneNote(nid),
            gate: &g,
            provider: "kimi",
            model: "k2",
            budget: 10_000,
            k: 5,
        };
        let ans = answer_question("how do I pay at the inn?", &deps)
            .await
            .unwrap();
        assert_eq!(ans.state, AnswerState::Grounded);
        assert!(ans.egress_log_id.is_some());
        let g = log.grounded.lock().unwrap();
        assert_eq!(g.len(), 1);
        assert!(g[0].1, "set_grounded(true) called");
    }

    #[tokio::test]
    async fn no_candidates_short_circuits_before_the_gate() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let deps = GenerateDeps {
            keyword: &NoKeyword,
            vectors: &OneVector(nid.to_string()),
            embedder: &FakeEmbed,
            notes: &EmptyRepo, // id retrieved but note missing → 0 candidates
            gate: &g,
            provider: "kimi",
            model: "k2",
            budget: 10_000,
            k: 5,
        };
        let ans = answer_question("anything", &deps).await.unwrap();
        assert_eq!(ans.state, AnswerState::NothingMatched);
        assert!(ans.egress_log_id.is_none());
        assert_eq!(fake.call_count(), 0, "no send");
        assert!(log.grounded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ungrounded_answer_sets_grounded_false() {
        // review #2: a SENT answer that isn't grounded must still persist the bit — as false.
        let nid = NoteId::new();
        // Valid JSON, but zero citations → Ungrounded.
        let fake = Arc::new(FakeLlmProvider::ok(
            r#"{"answer":"the sky is blue","cited_source_ids":[]}"#,
        ));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let deps = GenerateDeps {
            keyword: &NoKeyword,
            vectors: &OneVector(nid.to_string()),
            embedder: &FakeEmbed,
            notes: &OneNote(nid),
            gate: &g,
            provider: "kimi",
            model: "k2",
            budget: 10_000,
            k: 5,
        };
        let ans = answer_question("why is the sky blue?", &deps)
            .await
            .unwrap();
        assert_eq!(ans.state, AnswerState::Ungrounded);
        assert_eq!(fake.call_count(), 1, "it did send");
        let grounded = log.grounded.lock().unwrap();
        assert_eq!(grounded.len(), 1);
        assert!(
            !grounded[0].1,
            "set_grounded(false) persisted for the sent-but-ungrounded answer"
        );
    }

    #[test]
    fn prosemirror_body_is_flattened_to_text() {
        let doc = r#"{"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"Pay cash"},{"type":"text","text":" at the ryokan."}]},
            {"type":"paragraph","content":[{"type":"text","text":"Checkout is 10am."}]}
        ]}"#;
        assert_eq!(
            note_body_to_text(doc),
            "Pay cash  at the ryokan. Checkout is 10am."
        );
    }

    #[test]
    fn plain_text_body_passes_through_unchanged() {
        // Not a ProseMirror doc (no type:"doc") → returned verbatim.
        assert_eq!(note_body_to_text("just plain text"), "just plain text");
        assert_eq!(
            note_body_to_text(r#"{"type":"other"}"#),
            r#"{"type":"other"}"#
        );
    }
}
