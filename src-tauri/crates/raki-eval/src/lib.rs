//! Retrieval evaluation harness: load a taxonomy-tagged golden set, build a fresh
//! in-memory index, run keyword + vector retrieval, and score per category. v1 eval
//! is a bootstrap + regression tripwire, NOT a statistically-meaningful benchmark.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CorpusNote {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct EvalQuery {
    pub query: String,
    pub category: String,
    /// "dev" (used while tuning) or "holdout" (run only by the gate). Defaults to "dev".
    #[serde(default = "default_set")]
    pub set: String,
    #[serde(default)]
    pub relevant_ids: Vec<String>,
    /// Optional graded relevance (fixture id → grade). Absent ⇒ binary; nDCG dormant.
    #[serde(default)]
    pub grades: HashMap<String, f64>,
}

fn default_set() -> String {
    "dev".to_string()
}

const CORPUS_JSON: &str = include_str!("../fixtures/corpus.json");
const QUERIES_JSON: &str = include_str!("../fixtures/queries.json");

pub fn load_corpus() -> Vec<CorpusNote> {
    serde_json::from_str(CORPUS_JSON).expect("corpus.json is valid")
}

pub fn load_queries() -> Vec<EvalQuery> {
    serde_json::from_str(QUERIES_JSON).expect("queries.json is valid")
}

use std::collections::HashSet;
use std::sync::Arc;

use raki_domain::{DomainError, EmbeddingProvider, Note, NoteRepository, VectorIndex};
use raki_retrieval::{
    average_precision_at_k, hybrid_search, ndcg_at_k, recall_at_k, reciprocal_rank, search,
    vector_search,
};
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex};

/// Mean metrics for one retrieval method over a set of (scored) queries.
#[derive(Debug, Clone, Copy, Default)]
pub struct MethodScores {
    pub recall: f64,
    pub map: f64,
    pub mrr: f64,
    /// Mean nDCG@k over graded queries only; None when none are graded.
    pub ndcg: Option<f64>,
    /// Mean recall@K_cov over coverage queries only; None when none are coverage.
    pub recall_cov: Option<f64>,
}

/// Per-category breakdown — the point of the taxonomy.
#[derive(Debug, Clone)]
pub struct CategoryReport {
    pub category: String,
    pub scored: usize,
    pub keyword: MethodScores,
    pub vector: MethodScores,
    pub hybrid: MethodScores,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub k: usize,
    pub overall_keyword: MethodScores,
    pub overall_vector: MethodScores,
    pub overall_hybrid: MethodScores,
    pub by_category: Vec<CategoryReport>,
    /// Categories with no relevant labels (e.g. `negative`): tracked, not scored in v1
    /// (true-negative precision needs a score threshold we don't have yet).
    pub unscored_categories: Vec<String>,
}

/// One method's outcome for one query: the ranked fixture ids (top-k) and the metrics.
#[derive(Debug, Clone)]
pub struct MethodResult {
    pub ranked: Vec<String>, // fixture ids, best-first, truncated to k
    pub scores: MethodScores,
}

/// Per-query detail for every method — the substrate for the audit (3a-i) and the
/// per-query snapshot gate (3a-ii).
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub query: String,
    pub category: String,
    pub set: String,
    pub keyword: MethodResult,
    pub vector: MethodResult,
    pub hybrid: MethodResult,
}

/// Everything one eval run produces.
#[derive(Debug, Clone)]
pub struct EvalRun {
    pub report: Report,
    pub per_query: Vec<QueryResult>,
}

const COVERAGE_K: usize = 10;

/// Build a fresh in-memory index from the golden set, embed every document directly,
/// then score keyword and vector retrieval per query. Returns aggregated metrics
/// plus per-query detail.
pub async fn run_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    let corpus = load_corpus();
    let queries = load_queries();

    let db = Database::open_in_memory()?;
    let repo = SqliteNoteRepository::new(db.clone());
    let keyword = SqliteKeywordIndex::new(db.clone());
    let vectors = SqliteVectorIndex::new(db.clone());

    let mut fixture_of: HashMap<String, String> = HashMap::new();
    for cn in &corpus {
        const DUMMY_EPOCH_MS: i64 = 1000;
        let note = Note::new(cn.title.clone(), cn.body.clone(), DUMMY_EPOCH_MS);
        let uuid = note.id.to_string();
        repo.upsert(&note).await?;
        let doc = format!("{}\n\n{}", cn.title, cn.body);
        let emb = embedder.embed(std::slice::from_ref(&doc)).await?;
        let emb = emb.first().ok_or_else(|| {
            DomainError::Provider("embedder returned empty batch for single doc".to_string())
        })?;
        vectors.upsert(&uuid, emb).await?;
        fixture_of.insert(uuid, cn.id.clone());
    }

    let mut per_query: Vec<QueryResult> = Vec::new();
    let mut unscored: HashSet<String> = HashSet::new();

    for q in &queries {
        if q.relevant_ids.is_empty() {
            unscored.insert(q.category.clone());
            continue;
        }
        let relevant: HashSet<String> = q.relevant_ids.iter().cloned().collect();
        let cov_k = if q.category == "coverage" {
            COVERAGE_K
        } else {
            k
        };

        let kw = to_fixture(
            &search(&keyword, &q.query, cov_k.max(k)).await?,
            &fixture_of,
        );
        let vc = to_fixture(
            &vector_search(&vectors, embedder.as_ref(), &q.query, cov_k.max(k)).await?,
            &fixture_of,
        );
        let hy = to_fixture(
            &hybrid_search(
                &keyword,
                &vectors,
                embedder.as_ref(),
                &q.query,
                cov_k.max(k),
            )
            .await?,
            &fixture_of,
        );

        per_query.push(QueryResult {
            query: q.query.clone(),
            category: q.category.clone(),
            set: q.set.clone(),
            keyword: MethodResult {
                scores: score_one(&kw, &relevant, k, q),
                ranked: truncate(&kw, k),
            },
            vector: MethodResult {
                scores: score_one(&vc, &relevant, k, q),
                ranked: truncate(&vc, k),
            },
            hybrid: MethodResult {
                scores: score_one(&hy, &relevant, k, q),
                ranked: truncate(&hy, k),
            },
        });
    }

    let report = aggregate(&per_query, &mut unscored, k);
    Ok(EvalRun { report, per_query })
}

fn truncate(ids: &[String], k: usize) -> Vec<String> {
    ids.iter().take(k).cloned().collect()
}

fn score_one(ranked: &[String], relevant: &HashSet<String>, k: usize, q: &EvalQuery) -> MethodScores {
    MethodScores {
        recall: recall_at_k(ranked, relevant, k).unwrap_or(0.0),
        map: average_precision_at_k(ranked, relevant, k).unwrap_or(0.0),
        mrr: reciprocal_rank(ranked, relevant).unwrap_or(0.0),
        ndcg: if q.grades.is_empty() {
            None
        } else {
            ndcg_at_k(ranked, &q.grades, k)
        },
        recall_cov: if q.category == "coverage" {
            recall_at_k(ranked, relevant, COVERAGE_K)
        } else {
            None
        },
    }
}

fn aggregate(per_query: &[QueryResult], unscored: &mut HashSet<String>, k: usize) -> Report {
    use std::collections::BTreeMap;
    let mut cats: BTreeMap<&str, Vec<&QueryResult>> = BTreeMap::new();
    for qr in per_query {
        cats.entry(qr.category.as_str()).or_default().push(qr);
    }
    let mut by_category = Vec::new();
    for (cat, qrs) in &cats {
        by_category.push(CategoryReport {
            category: cat.to_string(),
            scored: qrs.len(),
            keyword: mean_scores(qrs.iter().map(|q| q.keyword.scores)),
            vector: mean_scores(qrs.iter().map(|q| q.vector.scores)),
            hybrid: mean_scores(qrs.iter().map(|q| q.hybrid.scores)),
        });
    }
    let overall_keyword = mean_scores(per_query.iter().map(|q| q.keyword.scores));
    let overall_vector = mean_scores(per_query.iter().map(|q| q.vector.scores));
    let overall_hybrid = mean_scores(per_query.iter().map(|q| q.hybrid.scores));
    let mut unscored_categories: Vec<String> = unscored.drain().collect();
    unscored_categories.sort();
    Report {
        k,
        overall_keyword,
        overall_vector,
        overall_hybrid,
        by_category,
        unscored_categories,
    }
}

fn mean_scores(it: impl Iterator<Item = MethodScores>) -> MethodScores {
    let v: Vec<MethodScores> = it.collect();
    let n = v.len().max(1) as f64;
    let opt_mean = |f: &dyn Fn(&MethodScores) -> Option<f64>| {
        let present: Vec<f64> = v.iter().filter_map(f).collect();
        if present.is_empty() {
            None
        } else {
            Some(present.iter().sum::<f64>() / present.len() as f64)
        }
    };
    MethodScores {
        recall: v.iter().map(|s| s.recall).sum::<f64>() / n,
        map: v.iter().map(|s| s.map).sum::<f64>() / n,
        mrr: v.iter().map(|s| s.mrr).sum::<f64>() / n,
        ndcg: opt_mean(&|s| s.ndcg),
        recall_cov: opt_mean(&|s| s.recall_cov),
    }
}

fn to_fixture(
    uuids: &[String],
    fixture_of: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    uuids
        .iter()
        .filter_map(|u| fixture_of.get(u).cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use raki_ai::FakeEmbeddingProvider;
    use std::sync::Arc;

    #[tokio::test]
    async fn harness_scores_every_category_with_fake_embedder() {
        let run = run_eval(Arc::new(FakeEmbeddingProvider::new(384)), 5)
            .await
            .unwrap();
        let report = &run.report;
        assert_eq!(report.k, 5);
        assert!(!run.per_query.is_empty());
        assert!(run.per_query.iter().all(|q| q.keyword.ranked.len() <= 5));
        // Every scored query category appears, and metrics are in range.
        assert!(report
            .by_category
            .iter()
            .any(|c| c.category == "lexical-overlap"));
        assert!(report
            .by_category
            .iter()
            .any(|c| c.category == "buried-fact-in-long-note"));
        for c in &report.by_category {
            assert!(c.keyword.recall >= 0.0 && c.keyword.recall <= 1.0);
            assert!(c.vector.map >= 0.0 && c.vector.map <= 1.0);
        }
        // Hybrid is computed and in range for every scored category.
        for c in &report.by_category {
            assert!(c.hybrid.recall >= 0.0 && c.hybrid.recall <= 1.0);
        }
        assert!(report.overall_hybrid.recall >= 0.0 && report.overall_hybrid.recall <= 1.0);
        // Keyword must actually find the exact lexical-overlap matches (sanity that
        // the index is wired), independent of the fake embedder's meaningless vectors.
        let lex = report
            .by_category
            .iter()
            .find(|c| c.category == "lexical-overlap")
            .unwrap();
        assert!(
            lex.keyword.recall > 0.0,
            "keyword should retrieve exact-term matches"
        );
        let cluster = run
            .per_query
            .iter()
            .find(|q| q.category == "lexical-cluster")
            .unwrap();
        assert!(
            cluster.keyword.scores.ndcg.is_some(),
            "graded query must produce nDCG"
        );
        let lex = run
            .report
            .by_category
            .iter()
            .find(|c| c.category == "lexical-cluster")
            .unwrap();
        assert!(
            lex.keyword.ndcg.is_some(),
            "lexical-cluster aggregate carries nDCG"
        );
        let cov = run
            .per_query
            .iter()
            .find(|q| q.category == "coverage")
            .unwrap();
        assert!(
            cov.vector.scores.recall_cov.is_some(),
            "coverage query must produce recall@K_cov"
        );
    }

    #[test]
    fn fixtures_parse_and_reference_real_corpus_ids() {
        let corpus = load_corpus();
        let queries = load_queries();
        assert!(corpus.len() >= 20, "need a non-trivial corpus");
        assert!(queries.len() >= 8, "need queries across the taxonomy");

        let ids: std::collections::HashSet<&str> = corpus.iter().map(|n| n.id.as_str()).collect();
        for q in &queries {
            for r in &q.relevant_ids {
                assert!(
                    ids.contains(r.as_str()),
                    "query references unknown corpus id {r}"
                );
            }
        }
        // The mandatory falsifiable-chunking category must be present.
        assert!(
            queries
                .iter()
                .any(|q| q.category == "buried-fact-in-long-note"),
            "taxonomy must include buried-fact-in-long-note"
        );
    }

    #[test]
    fn every_query_has_a_valid_set_and_resolvable_ids() {
        let corpus = load_corpus();
        let queries = load_queries();
        let ids: std::collections::HashSet<&str> = corpus.iter().map(|n| n.id.as_str()).collect();
        for q in &queries {
            assert!(
                q.set == "dev" || q.set == "holdout",
                "query {:?} has invalid set {:?}",
                q.query,
                q.set
            );
            for r in &q.relevant_ids {
                assert!(
                    ids.contains(r.as_str()),
                    "{:?} references unknown id {r}",
                    q.query
                );
            }
            for gid in q.grades.keys() {
                assert!(
                    ids.contains(gid.as_str()),
                    "{:?} grades unknown id {gid}",
                    q.query
                );
            }
        }
        assert!(
            queries.iter().any(|q| q.set == "holdout"),
            "need a holdout set"
        );
        assert!(queries.iter().any(|q| q.set == "dev"), "need a dev set");
    }

    #[test]
    fn coverage_queries_have_many_relevant() {
        for q in load_queries() {
            if q.category == "coverage" {
                assert!(
                    q.relevant_ids.len() >= 4,
                    "coverage query {:?} should have a broad answer set",
                    q.query
                );
            }
        }
    }

    #[test]
    fn ordering_categories_carry_grades() {
        const ORDERING: &[&str] = &[
            "lexical-cluster",
            "dense-near-duplicate",
            "paraphrase-distractor",
        ];
        for q in load_queries() {
            if ORDERING.contains(&q.category.as_str()) {
                assert!(
                    !q.grades.is_empty(),
                    "ordering query {:?} must carry grades",
                    q.query
                );
            }
        }
    }
}
