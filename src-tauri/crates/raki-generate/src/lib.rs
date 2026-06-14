//! Grounded QA orchestration: retrieve → assemble → gate → answer → verify. Composes the leaf
//! crates (the dependency rule forbids a leaf from doing this — see spec "Crate placement").

pub use raki_domain::{evaluate, Answer, AnswerState, EgressPreview};

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

/// Non-egress vs egress failures stay distinguishable (spec C2).
#[derive(Debug)]
pub enum GenerateError {
    Egress(EgressError),
    Domain(DomainError),
}

use raki_domain::CompletionRequest;
use raki_memory::{assemble_context, AssembledContext, Candidate};
use raki_retrieval::hybrid_search;

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
    let context_ids: std::collections::HashSet<SourceId> =
        ctx.egress.source_ids.iter().cloned().collect();
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
        source_titles,
    }))
}
