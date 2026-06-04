//! Context assembly: the single, deterministic, token-budgeted bundle a model sees.
//! Pure and testable — no IO, no providers.

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
}

/// Rough token estimate (~4 chars per token), never zero for non-empty text.
fn estimate_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

/// Greedily include highest-scored candidates until the token budget is exhausted.
pub fn assemble_context(candidates: &[Candidate], budget: usize) -> AssembledContext {
    let mut ranked: Vec<&Candidate> = candidates.iter().collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
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

    AssembledContext {
        items,
        total_tokens,
        budget,
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
        // budget fits exactly one 4-char item (~1 token).
        let ctx = assemble_context(&candidates, 1);
        assert_eq!(ctx.items.len(), 1);
        assert_eq!(ctx.items[0].source_id, "high");
        assert!(ctx.total_tokens <= ctx.budget);
    }

    #[test]
    fn includes_everything_when_budget_is_large() {
        let candidates = vec![cand("a", "x", 0.5), cand("b", "y", 0.4)];
        let ctx = assemble_context(&candidates, 10_000);
        assert_eq!(ctx.items.len(), 2);
    }
}
