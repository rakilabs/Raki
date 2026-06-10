//! Renders the chunking *design* baseline: whole-note vs chunked arms, per-arm and per-category
//! MAP deltas, the buried-fact winner, model ids, and the honesty/P1 header. Pure — no model, no
//! I/O — so it is unit-testable; the `chunk-eval` binary is thin glue over it. Synthetic numbers
//! settle DESIGN only; the binding verdict is real-notes-gated (chunking spec D8).

use crate::Report;

/// Render the recorded markdown for the synthetic chunking comparison, and name the winning arm.
///
/// `arms` is `(label, chunked_report)` for each `prefix × rollup`. The **winning arm** is the one
/// whose buried-fact-category **vector MAP** delta vs `whole` is greatest (vector is the recall-stage
/// signal chunking most directly moves; ties keep the first arm). Falls back to overall-vector MAP
/// delta if no category name contains "buried-fact".
pub fn render_chunking_baseline(whole: &Report, arms: &[(String, Report)], models: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    let _ = writeln!(out, "# Chunking design baseline (synthetic, k={})", whole.k);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> **Directional, design-settling only.** The synthetic corpus is small and recall \
saturates, so the ranking signal lives in **MAP**. The **binding** chunking verdict is \
real-notes-gated (chunking spec D8: +0.05 Success@3 on the long stratum, by 2026-09-06) — its \
enabler is roadmap Track B **P1**. This file records *which chunk design* to carry, not whether \
to ship."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "models: {models}");
    let _ = writeln!(out);

    // Winner: greatest buried-fact-category vector MAP delta (fallback: overall vector MAP delta).
    let whole_bf = buried_fact_vector_map(whole);
    let mut winner: Option<(&str, f64)> = None;
    for (label, rep) in arms {
        let delta = match (buried_fact_vector_map(rep), whole_bf) {
            (Some(c), Some(w)) => c - w,
            _ => rep.overall_vector.map - whole.overall_vector.map,
        };
        if winner.is_none_or(|(_, best)| delta > best) {
            winner = Some((label.as_str(), delta));
        }
    }
    if let Some((label, delta)) = winner {
        let _ = writeln!(
            out,
            "**Winning arm (buried-fact vector ΔMAP): `{label}` (Δ {delta:+.3})**"
        );
        let _ = writeln!(out);
    }

    // Per-arm overall deltas: vector + reranked headlined, hybrid demoted (deployment-risk).
    let _ = writeln!(
        out,
        "| arm | vec ΔMAP | reranked ΔMAP | hybrid ΔMAP (deploy-risk) |"
    );
    let _ = writeln!(
        out,
        "|-----|---------:|--------------:|-------------------------:|"
    );
    for (label, rep) in arms {
        let _ = writeln!(
            out,
            "| {label} | {:+.3} | {:+.3} | {:+.3} |",
            rep.overall_vector.map - whole.overall_vector.map,
            rep.overall_reranked.map - whole.overall_reranked.map,
            rep.overall_hybrid.map - whole.overall_hybrid.map,
        );
    }
    let _ = writeln!(out);

    // Per-category deltas (buried-fact / coreference / list controls) for each arm.
    for (label, rep) in arms {
        let _ = writeln!(out, "### {label} — per-category ΔMAP (vs whole)");
        for cat in &rep.by_category {
            if let Some(w) = whole
                .by_category
                .iter()
                .find(|c| c.category == cat.category)
            {
                let _ = writeln!(
                    out,
                    "- [{}] vec {:+.3} | reranked {:+.3}",
                    cat.category,
                    cat.vector.map - w.vector.map,
                    cat.reranked.map - w.reranked.map,
                );
            }
        }
        let _ = writeln!(out);
    }

    out
}

/// Buried-fact-category vector MAP for a report, if such a category exists.
fn buried_fact_vector_map(report: &Report) -> Option<f64> {
    report
        .by_category
        .iter()
        .find(|c| c.category.contains("buried-fact"))
        .map(|c| c.vector.map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CategoryReport, MethodScores, Report};

    fn ms(map: f64) -> MethodScores {
        MethodScores {
            recall: 1.0,
            map,
            mrr: map,
            ndcg: None,
            recall_cov: None,
        }
    }

    /// A report whose buried-fact category has the given vector MAP.
    fn report(buried_fact_vec_map: f64) -> Report {
        Report {
            k: 10,
            overall_keyword: ms(0.50),
            overall_vector: ms(0.50),
            overall_hybrid: ms(0.50),
            overall_reranked: ms(0.50),
            by_category: vec![CategoryReport {
                category: "buried-fact-long-note".into(),
                scored: 5,
                keyword: ms(0.40),
                vector: ms(buried_fact_vec_map),
                hybrid: ms(0.40),
                reranked: ms(0.40),
            }],
            unscored_categories: vec![],
        }
    }

    #[test]
    fn names_best_buried_fact_arm_and_emits_sections() {
        let whole = report(0.40);
        let arms = vec![
            ("bare/min-rank".to_string(), report(0.45)),  // +0.05
            ("title/min-rank".to_string(), report(0.60)), // +0.20  ← winner
            ("title+head/score-max".to_string(), report(0.42)), // +0.02
        ];
        let md = render_chunking_baseline(
            &whole,
            &arms,
            "bge-small-en-v1.5 / jina-reranker-v1-turbo-en",
        );

        assert!(md.contains("Winning arm"), "has a winner line");
        assert!(
            md.contains("`title/min-rank`"),
            "best buried-fact ΔMAP arm wins"
        );
        assert!(
            md.contains("buried-fact-long-note"),
            "per-category rows present"
        );
        assert!(md.contains("models: bge-small"), "model line present");
        assert!(md.contains("P1"), "honesty/P1 header present");
        assert!(
            md.contains("deploy-risk"),
            "hybrid demoted as deployment-risk"
        );
    }
}
