//! Context assembly: the single, deterministic, token-budgeted bundle a model sees.
//! Pure and testable — no IO, no providers.

use raki_domain::{EgressDecision, SourceId};

/// A retrieval candidate competing for space in the context window.
pub struct Candidate {
    pub source_id: String,
    pub text: String,
    pub score: f64,
}

/// One included item, with the reason it earned its place.
pub struct ContextItem {
    pub source_id: String,
    pub text: String,
    pub token_estimate: usize,
    pub reason: String,
}

/// The assembled, budgeted context.
pub struct AssembledContext {
    pub items: Vec<ContextItem>,
    pub total_tokens: usize,
    pub budget: usize,
    pub egress: EgressDecision,
}

/// Rough token estimate (~4 chars per token), never zero for non-empty text.
fn estimate_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

/// Derive the egress metadata for an assembled set of items aimed at `provider`/`model`.
pub fn egress_of(items: &[ContextItem], provider: &str, model: &str) -> EgressDecision {
    EgressDecision {
        provider: provider.to_string(),
        model: model.to_string(),
        source_ids: items
            .iter()
            .map(|i| SourceId(i.source_id.clone()))
            .collect(),
        total_tokens: items.iter().map(|i| i.token_estimate).sum(),
    }
}

/// Greedily include highest-scored candidates until the token budget is exhausted.
pub fn assemble_context(
    candidates: &[Candidate],
    budget: usize,
    provider: &str,
    model: &str,
) -> AssembledContext {
    let mut ranked: Vec<&Candidate> = candidates.iter().collect();
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.source_id.cmp(&b.source_id))
    });

    let mut items = Vec::new();
    let mut total_tokens = 0usize;
    for c in ranked {
        let tokens = estimate_tokens(&c.text);
        if total_tokens + tokens > budget {
            continue;
        }
        total_tokens += tokens;
        items.push(ContextItem {
            source_id: c.source_id.clone(),
            text: c.text.clone(),
            token_estimate: tokens,
            reason: format!("retrieval score {:.3}", c.score),
        });
    }

    let egress = egress_of(&items, provider, model);
    AssembledContext {
        items,
        total_tokens,
        budget,
        egress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: &str, text: &str, score: f64) -> Candidate {
        Candidate {
            source_id: id.to_string(),
            text: text.to_string(),
            score,
        }
    }

    #[test]
    fn picks_highest_scored_first_within_budget() {
        let candidates = vec![cand("low", "aaaa", 0.1), cand("high", "bbbb", 0.9)];
        let ctx = assemble_context(&candidates, 1, "kimi", "k2");
        assert_eq!(ctx.items.len(), 1);
        assert_eq!(ctx.items[0].source_id, "high");
        assert!(ctx.total_tokens <= ctx.budget);
        // egress mirrors the included items exactly.
        assert_eq!(ctx.egress.source_ids, vec![SourceId("high".to_string())]);
        assert_eq!(ctx.egress.total_tokens, ctx.total_tokens);
        assert_eq!(ctx.egress.provider, "kimi");
    }

    #[test]
    fn includes_everything_when_budget_is_large() {
        let candidates = vec![cand("a", "x", 0.5), cand("b", "y", 0.4)];
        let ctx = assemble_context(&candidates, 10_000, "kimi", "k2");
        assert_eq!(ctx.items.len(), 2);
        assert_eq!(ctx.egress.source_ids.len(), 2);
    }

    #[test]
    fn egress_of_is_metadata_of_included_items() {
        let items = vec![ContextItem {
            source_id: "n1".into(),
            text: "hello".into(),
            token_estimate: 3,
            reason: "x".into(),
        }];
        let e = egress_of(&items, "kimi", "k2");
        assert_eq!(e.source_ids, vec![SourceId("n1".to_string())]);
        assert_eq!(e.total_tokens, 3);
    }
}
