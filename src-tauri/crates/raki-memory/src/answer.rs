//! Grounded answer orchestration: retrieve → assemble → gate → answer → verify.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raki_domain::{
    body_to_text, evaluate, Answer, AnswerState, CompletionRequest, DomainError, EgressDenied,
    EgressError, EgressPreview, EmbeddingProvider, GatedLlmProvider, KeywordIndex, NoteRepository,
    QueryRewriter, SourceId, VectorIndex,
};
use raki_retrieval::hybrid_search;

use crate::context::{assemble_context, AssembledContext, Candidate};

#[derive(Clone, Debug)]
pub struct AnswerConfig {
    pub provider: String,
    pub model: String,
    pub k: usize,
    pub budget: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("egress denied or failed: {0}")]
    Egress(#[from] EgressError),
    #[error("domain error: {0}")]
    Domain(#[from] DomainError),
}

#[derive(Debug)]
pub enum AnswerResult {
    Answer(Answer),
    NeedsConsent(EgressPreview),
}

pub struct AnswerService {
    keyword: Arc<dyn KeywordIndex>,
    vectors: Arc<dyn VectorIndex>,
    embedder: Arc<dyn EmbeddingProvider>,
    notes: Arc<dyn NoteRepository>,
    gate: Arc<dyn GatedLlmProvider>,
    config: AnswerConfig,
}

impl AnswerService {
    pub fn new(
        keyword: Arc<dyn KeywordIndex>,
        vectors: Arc<dyn VectorIndex>,
        embedder: Arc<dyn EmbeddingProvider>,
        notes: Arc<dyn NoteRepository>,
        gate: Arc<dyn GatedLlmProvider>,
        config: AnswerConfig,
    ) -> Self {
        Self {
            keyword,
            vectors,
            embedder,
            notes,
            gate,
            config,
        }
    }

    pub async fn answer(
        &self,
        query: &str,
        rewriter: Option<&dyn QueryRewriter>,
    ) -> Result<AnswerResult, GenerateError> {
        let Some((ctx, titles)) = self.assemble(query, rewriter).await? else {
            return Ok(AnswerResult::Answer(Answer {
                state: AnswerState::NothingMatched,
                text: "No relevant notes found.".into(),
                cited_ids: vec![],
                egress_log_id: None,
            }));
        };
        match self.send(&ctx, query).await {
            Ok(ans) => Ok(AnswerResult::Answer(ans)),
            Err(GenerateError::Egress(EgressError::Denied(EgressDenied::ConsentRequired))) => {
                Ok(AnswerResult::NeedsConsent(self.preview_from_context(&ctx, &titles)))
            }
            Err(e) => Err(e),
        }
    }

    fn preview_from_context(
        &self,
        ctx: &AssembledContext,
        titles: &HashMap<String, String>,
    ) -> EgressPreview {
        let source_titles = ctx
            .egress
            .source_ids
            .iter()
            .map(|s| titles.get(&s.0).cloned().unwrap_or_else(|| s.0.clone()))
            .collect();
        EgressPreview {
            provider: self.config.provider.clone(),
            source_titles,
        }
    }

    async fn assemble(
        &self,
        query: &str,
        rewriter: Option<&dyn QueryRewriter>,
    ) -> Result<Option<(AssembledContext, HashMap<String, String>)>, GenerateError> {
        let ids = hybrid_search(
            self.keyword.as_ref(),
            self.vectors.as_ref(),
            self.embedder.as_ref(),
            rewriter,
            query,
            self.config.k,
        )
        .await
        .map_err(GenerateError::Domain)?;

        let mut candidates = Vec::new();
        let mut titles = HashMap::new();
        for (rank, id) in ids.iter().enumerate() {
            if let Some(note) = self.notes.get(id).await.map_err(GenerateError::Domain)? {
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
        let ctx = assemble_context(
            &candidates,
            self.config.budget,
            &self.config.provider,
            &self.config.model,
        );
        Ok(Some((ctx, titles)))
    }

    async fn send(&self, ctx: &AssembledContext, query: &str) -> Result<Answer, GenerateError> {
        let req = CompletionRequest {
            system: Some(build_system_prompt(ctx)),
            prompt: query.to_string(),
            max_tokens: None,
        };
        let (completion, log_id) = self
            .gate
            .complete_gated(&ctx.egress, req)
            .await
            .map_err(GenerateError::Egress)?;
        let context_ids: HashSet<SourceId> = ctx.egress.source_ids.iter().cloned().collect();
        let (state, text, cited_ids) = evaluate(&completion.text, &context_ids);
        if let Some(id) = log_id {
            self.gate
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
}

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

#[cfg(test)]
mod flow_tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use raki_ai::testing::FakeLlmProvider;
    use raki_ai::AuditGate;
    use raki_domain::testing::FixedClock;
    use raki_domain::{
        DomainError, EgressLog, EgressLogId, EgressRecord, EgressSettings, Embedding, KeywordHit,
        Note, NoteId, VectorHit,
    };

    // --- fakes (impl domain ports) ---
    struct OneVector(String);
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

    fn gate(
        inner: Arc<dyn raki_domain::LlmProvider>,
        log: Arc<SpyLog>,
    ) -> Arc<dyn GatedLlmProvider> {
        Arc::new(AuditGate::new(
            inner,
            Arc::new(ConsentedSettings),
            log,
            Arc::new(FixedClock(1000)),
        ))
    }

    fn service(
        gate: Arc<dyn GatedLlmProvider>,
        notes: Arc<dyn NoteRepository>,
        vectors: Arc<dyn VectorIndex>,
    ) -> AnswerService {
        AnswerService::new(
            Arc::new(NoKeyword),
            vectors,
            Arc::new(FakeEmbed),
            notes,
            gate,
            AnswerConfig {
                provider: "kimi".into(),
                model: "k2".into(),
                k: 5,
                budget: 10_000,
            },
        )
    }

    #[derive(Default)]
    struct ToggleSettings {
        consented: Mutex<HashSet<String>>,
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
        Arc::new(AuditGate::new(
            inner,
            settings,
            log,
            Arc::new(FixedClock(1000)),
        ))
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
    async fn grounded_answer_sets_grounded_true() {
        let nid = NoteId::new();
        let reply = r#"{"answer":"Pay cash.","cited_source_ids":["IDPLACEHOLDER"],"insufficient_context":false}"#
            .replace("IDPLACEHOLDER", &nid.to_string());
        let fake = Arc::new(FakeLlmProvider::ok(&reply));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let notes: Arc<dyn NoteRepository> = Arc::new(OneNote(nid));
        let vectors: Arc<dyn VectorIndex> = Arc::new(OneVector(nid.to_string()));
        let svc = service(g, notes, vectors);
        let result = svc.answer("how do I pay at the inn?", None).await.unwrap();
        let AnswerResult::Answer(ans) = result else {
            panic!("expected Answer, got {result:?}");
        };
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
        let notes: Arc<dyn NoteRepository> = Arc::new(EmptyRepo);
        let vectors: Arc<dyn VectorIndex> = Arc::new(OneVector(nid.to_string()));
        let svc = service(g, notes, vectors);
        let result = svc.answer("anything", None).await.unwrap();
        let AnswerResult::Answer(ans) = result else {
            panic!("expected Answer, got {result:?}");
        };
        assert_eq!(ans.state, AnswerState::NothingMatched);
        assert!(ans.egress_log_id.is_none());
        assert_eq!(fake.call_count(), 0, "no send");
        assert!(log.grounded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ungrounded_answer_sets_grounded_false() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok(
            r#"{"answer":"the sky is blue","cited_source_ids":[]}"#,
        ));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let notes: Arc<dyn NoteRepository> = Arc::new(OneNote(nid));
        let vectors: Arc<dyn VectorIndex> = Arc::new(OneVector(nid.to_string()));
        let svc = service(g, notes, vectors);
        let result = svc.answer("why is the sky blue?", None).await.unwrap();
        let AnswerResult::Answer(ans) = result else {
            panic!("expected Answer, got {result:?}");
        };
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
    async fn answer_returns_needs_consent_when_not_consented() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let settings = Arc::new(ToggleSettings::default());
        let g = toggle_gate(fake.clone(), settings, log.clone());
        let notes: Arc<dyn NoteRepository> = Arc::new(OneNote(nid));
        let vectors: Arc<dyn VectorIndex> = Arc::new(OneVector(nid.to_string()));
        let svc = service(g, notes, vectors);
        let result = svc.answer("how do I pay?", None).await.unwrap();
        let AnswerResult::NeedsConsent(preview) = result else {
            panic!("expected NeedsConsent, got {result:?}");
        };
        assert_eq!(preview.provider, "kimi");
        assert_eq!(preview.source_titles, vec!["Trip".to_string()]);
        assert_eq!(fake.call_count(), 0, "no send while denied");
    }

    #[tokio::test]
    async fn answer_is_nothing_matched_when_repo_empty() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log);
        let notes: Arc<dyn NoteRepository> = Arc::new(EmptyRepo);
        let vectors: Arc<dyn VectorIndex> = Arc::new(OneVector(nid.to_string()));
        let svc = service(g, notes, vectors);
        let result = svc.answer("x", None).await.unwrap();
        let AnswerResult::Answer(ans) = result else {
            panic!("expected Answer, got {result:?}");
        };
        assert_eq!(ans.state, AnswerState::NothingMatched);
    }

    #[tokio::test]
    async fn rewriter_is_forwarded_to_hybrid_search() {
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log);
        let notes: Arc<dyn NoteRepository> = Arc::new(OneNote(NoteId::new()));
        let vectors: Arc<dyn VectorIndex> = Arc::new(OneVector(NoteId::new().to_string()));
        let svc = service(g, notes, vectors);
        // Wiring check; result doesn't matter.
        let _ = svc
            .answer("vague query", Some(&FakeRewriter("explicit keyword")))
            .await;
    }

    #[tokio::test]
    async fn answer_succeeds_after_consent_is_granted() {
        let nid = NoteId::new();
        let reply = r#"{"answer":"Pay cash.","cited_source_ids":["IDPLACEHOLDER"],"insufficient_context":false}"#
            .replace("IDPLACEHOLDER", &nid.to_string());
        let fake = Arc::new(FakeLlmProvider::ok(&reply));
        let log = Arc::new(SpyLog::default());
        let settings = Arc::new(ToggleSettings::default());
        let g = toggle_gate(fake.clone(), settings.clone(), log.clone());
        let notes: Arc<dyn NoteRepository> = Arc::new(OneNote(nid));
        let vectors: Arc<dyn VectorIndex> = Arc::new(OneVector(nid.to_string()));
        let svc = service(g, notes, vectors);

        let result = svc.answer("how do I pay?", None).await.unwrap();
        let AnswerResult::NeedsConsent(_) = result else {
            panic!("expected NeedsConsent, got {result:?}");
        };
        assert_eq!(fake.call_count(), 0, "no send while denied");

        settings.grant("kimi").await.unwrap();

        let result = svc.answer("how do I pay?", None).await.unwrap();
        let AnswerResult::Answer(ans) = result else {
            panic!("expected Answer, got {result:?}");
        };
        assert_eq!(ans.state, AnswerState::Grounded);
        assert_eq!(fake.call_count(), 1, "send after consent");
    }
}
