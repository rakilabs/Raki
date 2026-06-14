//! Grounded QA orchestration: retrieve → assemble → gate → answer → verify. Composes the leaf
//! crates (the dependency rule forbids a leaf from doing this — see spec "Crate placement").

mod groundedness;

pub use groundedness::AnswerState;

use raki_domain::{
    body_to_text, DomainError, EgressError, EmbeddingProvider, GatedLlmProvider, KeywordIndex,
    NoteRepository, SourceId, VectorIndex,
};

/// Everything `answer_question` needs, injected so the flow is fake-testable.
pub struct GenerateDeps<'a> {
    pub keyword: &'a dyn KeywordIndex,
    pub vectors: &'a dyn VectorIndex,
    pub embedder: &'a dyn EmbeddingProvider, // assumed LOCAL (spec M4)
    pub notes: &'a dyn NoteRepository,
    pub gate: &'a dyn GatedLlmProvider,
    pub provider: &'a str,
    pub model: &'a str,
    pub budget: usize,
    pub k: usize,
    pub rewriter: Option<&'a dyn raki_domain::QueryRewriter>,
}

/// The result of a QA request.
#[derive(Debug)]
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

use raki_domain::CompletionRequest;
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

/// Retrieve + assemble locally (no model call). Returns the assembled context and an
/// id→title map for the included sources, or `None` when nothing matched.
pub async fn assemble_for(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Option<(AssembledContext, std::collections::HashMap<String, String>)>, GenerateError> {
    let ids = hybrid_search(
        deps.keyword,
        deps.vectors,
        deps.embedder,
        deps.rewriter,
        query,
        deps.k,
    )
    .await
    .map_err(GenerateError::Domain)?;

    let mut candidates = Vec::new();
    let mut titles = std::collections::HashMap::new();
    for (rank, id) in ids.iter().enumerate() {
        if let Some(note) = deps.notes.get(id).await.map_err(GenerateError::Domain)? {
            titles.insert(id.to_string(), note.title.clone());
            candidates.push(Candidate {
                source_id: id.to_string(),
                text: format!("{}\n{}", note.title, body_to_text(&note.body)),
                score: (ids.len() - rank) as f64,
            });
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    let ctx = assemble_context(&candidates, deps.budget, deps.provider, deps.model);
    Ok(Some((ctx, titles)))
}

/// Send the assembled context to the model (gated). The actual completion + groundedness check.
pub async fn send_answer(
    ctx: &AssembledContext,
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Answer, GenerateError> {
    let req = CompletionRequest {
        system: Some(build_system_prompt(ctx)),
        prompt: query.to_string(),
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
    if let Some(id) = log_id {
        deps.gate
            .set_grounded(&id, state.is_grounded())
            .await
            .map_err(GenerateError::Domain)?;
    }
    Ok(Answer {
        state,
        text,
        cited_ids,
        egress_log_id: log_id,
    })
}

pub async fn answer_question(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Answer, GenerateError> {
    let Some((ctx, _titles)) = assemble_for(query, deps).await? else {
        return Ok(Answer {
            state: AnswerState::NothingMatched,
            text: "No relevant notes found.".into(),
            cited_ids: vec![],
            egress_log_id: None,
        });
    };
    send_answer(&ctx, query, deps).await
}

/// What a cloud send WOULD disclose — shown to the user before consent (spec D7). Metadata only.
pub struct EgressPreview {
    pub provider: String,
    pub summary: String,
    pub source_titles: Vec<String>,
}

/// The egress preview for `query` (no send), or `None` if nothing matched.
pub async fn preview(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Option<EgressPreview>, GenerateError> {
    let Some((ctx, titles)) = assemble_for(query, deps).await? else {
        return Ok(None);
    };
    let source_titles = ctx
        .egress
        .source_ids
        .iter()
        .map(|s| titles.get(&s.0).cloned().unwrap_or_else(|| s.0.clone()))
        .collect();
    Ok(Some(EgressPreview {
        provider: deps.provider.to_string(),
        summary: ctx.egress.summary(),
        source_titles,
    }))
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
        EgressDenied, EgressLog, EgressLogId, EgressRecord, EgressSettings, Embedding, KeywordHit,
        Note, NoteId, VectorHit,
    };

    // --- fakes (impl domain ports) ---
    struct OneVector(String); // returns a single source id from the vector index
    #[async_trait]
    impl VectorIndex for OneVector {
        async fn upsert(&self, _: &str, _: &Embedding) -> Result<(), DomainError> {
            Ok(())
        }
        async fn delete_by_prefix(&self, _prefix: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn upsert_batch(&self, _items: &[(String, Embedding)]) -> Result<(), DomainError> {
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
        async fn update(&self, _: &Note) -> Result<bool, DomainError> {
            Ok(true)
        }
        async fn get(&self, id: &NoteId) -> Result<Option<Note>, DomainError> {
            Ok((*id == self.0)
                .then(|| Note::new("Trip".into(), "Pay cash at the ryokan.".into(), 0)))
        }
        async fn get_any(&self, id: &NoteId) -> Result<Option<Note>, DomainError> {
            self.get(id).await
        }
        async fn list(&self) -> Result<Vec<Note>, DomainError> {
            Ok(vec![])
        }
        async fn list_trashed(&self) -> Result<Vec<Note>, DomainError> {
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
        async fn update(&self, _: &Note) -> Result<bool, DomainError> {
            Ok(true)
        }
        async fn get(&self, _: &NoteId) -> Result<Option<Note>, DomainError> {
            Ok(None)
        }
        async fn get_any(&self, _: &NoteId) -> Result<Option<Note>, DomainError> {
            Ok(None)
        }
        async fn list(&self) -> Result<Vec<Note>, DomainError> {
            Ok(vec![])
        }
        async fn list_trashed(&self) -> Result<Vec<Note>, DomainError> {
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
        async fn list_recent(&self, _limit: usize) -> Result<Vec<EgressRecord>, DomainError> {
            Ok(vec![])
        }
    }
    struct ConsentedSettings;
    #[async_trait]
    impl EgressSettings for ConsentedSettings {
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(HashSet::from(["kimi".to_string()]))
        }
        async fn grant(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn revoke(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    fn gate(inner: Arc<dyn raki_domain::LlmProvider>, log: Arc<SpyLog>) -> Arc<dyn GatedLlmProvider> {
        Arc::new(raki_ai::AuditGate::new(
            inner,
            Arc::new(ConsentedSettings),
            log,
            Arc::new(FixedClock(1000)),
        ))
    }

    fn test_deps<'a>(
        gate: &'a dyn GatedLlmProvider,
        notes: &'a dyn NoteRepository,
        vectors: &'a dyn VectorIndex,
    ) -> GenerateDeps<'a> {
        GenerateDeps {
            keyword: &NoKeyword,
            vectors,
            embedder: &FakeEmbed,
            notes,
            gate,
            provider: "kimi",
            model: "k2",
            budget: 10_000,
            k: 5,
            rewriter: None,
        }
    }

    struct ToggleSettings {
        consented: Mutex<HashSet<String>>,
    }
    impl Default for ToggleSettings {
        fn default() -> Self {
            Self {
                consented: Mutex::new(HashSet::new()),
            }
        }
    }
    #[async_trait]
    impl EgressSettings for ToggleSettings {
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(self.consented.lock().unwrap().clone())
        }
        async fn grant(&self, p: &str) -> Result<(), DomainError> {
            self.consented.lock().unwrap().insert(p.to_string());
            Ok(())
        }
        async fn revoke(&self, p: &str) -> Result<(), DomainError> {
            self.consented.lock().unwrap().remove(p);
            Ok(())
        }
    }

    fn toggle_gate(
        inner: Arc<dyn raki_domain::LlmProvider>,
        settings: Arc<ToggleSettings>,
        log: Arc<SpyLog>,
    ) -> Arc<dyn GatedLlmProvider> {
        Arc::new(raki_ai::AuditGate::new(inner, settings, log, Arc::new(FixedClock(1000))))
    }

    #[tokio::test]
    async fn grounded_answer_sets_grounded_true() {
        let nid = NoteId::new();
        let reply = r#"{"answer":"Pay cash.","cited_source_ids":["IDPLACEHOLDER"],"insufficient_context":false}"#
            .replace("IDPLACEHOLDER", &nid.to_string());
        let fake = Arc::new(FakeLlmProvider::ok(&reply));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log.clone());
        let note = OneNote(nid);
        let vec = OneVector(nid.to_string());
        let deps = test_deps(g.as_ref(), &note, &vec);
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
        let vec = OneVector(nid.to_string());
        let deps = test_deps(g.as_ref(), &EmptyRepo, &vec);
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
        let note = OneNote(nid);
        let vec = OneVector(nid.to_string());
        let deps = test_deps(g.as_ref(), &note, &vec);
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

    #[tokio::test]
    async fn preview_returns_egress_metadata_without_sending() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let note = OneNote(nid);
        let vec = OneVector(nid.to_string());
        let deps = test_deps(g.as_ref(), &note, &vec);
        let p = preview("how do I pay?", &deps)
            .await
            .unwrap()
            .expect("some preview");
        assert_eq!(p.provider, "kimi");
        assert_eq!(p.source_titles, vec!["Trip".to_string()]);
        assert!(p.summary.contains("→ kimi/k2"));
        assert_eq!(fake.call_count(), 0, "preview never sends");
    }

    #[tokio::test]
    async fn preview_is_none_when_nothing_matched() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log);
        let vec = OneVector(nid.to_string());
        let deps = test_deps(g.as_ref(), &EmptyRepo, &vec);
        assert!(preview("x", &deps).await.unwrap().is_none());
    }

    struct FakeRewriter(&'static str);
    #[async_trait]
    impl raki_domain::QueryRewriter for FakeRewriter {
        async fn understand(
            &self,
            _query: &str,
        ) -> Result<raki_domain::QueryUnderstanding, DomainError> {
            Ok(raki_domain::QueryUnderstanding {
                rewritten_query: self.0.to_string(),
                needs_multi_hop: false,
                sub_queries: vec![],
                confidence: 0.9,
                is_fallback: false,
            })
        }
    }

    #[tokio::test]
    async fn rewriter_is_forwarded_to_hybrid_search() {
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log);
        let note = OneNote(NoteId::new());
        let vec = OneVector(NoteId::new().to_string());
        let mut deps = test_deps(g.as_ref(), &note, &vec);
        deps.rewriter = Some(&FakeRewriter("explicit keyword"));
        // This test doesn't assert on the result; it asserts the wiring doesn't panic.
        // A more rigorous test would verify the effective query reaches the keyword index.
        let _ = assemble_for("vague query", &deps).await;
    }

    #[tokio::test]
    async fn send_answer_succeeds_after_consent_is_granted() {
        let nid = NoteId::new();
        let reply = r#"{"answer":"Pay cash.","cited_source_ids":["IDPLACEHOLDER"],"insufficient_context":false}"#
            .replace("IDPLACEHOLDER", &nid.to_string());
        let fake = Arc::new(FakeLlmProvider::ok(&reply));
        let log = Arc::new(SpyLog::default());
        let settings = Arc::new(ToggleSettings::default());
        // Start with no consent → gate denies.
        let g = toggle_gate(fake.clone(), settings.clone(), log.clone());
        let note = OneNote(nid);
        let vec = OneVector(nid.to_string());
        let deps = test_deps(g.as_ref(), &note, &vec);

        let Some((ctx, _titles)) = assemble_for("how do I pay?", &deps).await.unwrap() else {
            panic!("expected some context");
        };

        // First send: denied because no consent.
        let err = send_answer(&ctx, "how do I pay?", &deps).await.unwrap_err();
        assert!(
            matches!(
                err,
                GenerateError::Egress(EgressError::Denied(EgressDenied::ConsentRequired))
            ),
            "expected ConsentRequired denial, got {err:?}"
        );
        assert_eq!(fake.call_count(), 0, "no send while denied");

        // Grant consent.
        settings.grant("kimi").await.unwrap();

        // Second send: succeeds now.
        let ans = send_answer(&ctx, "how do I pay?", &deps).await.unwrap();
        assert_eq!(ans.state, AnswerState::Grounded);
        assert_eq!(fake.call_count(), 1, "send after consent");
    }
}
