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
    #[serde(default)]
    pub relevant_ids: Vec<String>,
    /// Optional graded relevance (fixture id → grade). Absent ⇒ binary; nDCG dormant.
    #[serde(default)]
    pub grades: HashMap<String, f64>,
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
use raki_retrieval::{average_precision_at_k, recall_at_k, reciprocal_rank, search, vector_search};
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex};

/// Mean metrics for one retrieval method over a set of (scored) queries.
#[derive(Debug, Clone, Copy, Default)]
pub struct MethodScores {
    pub recall: f64,
    pub map: f64,
    pub mrr: f64,
}

/// Per-category breakdown — the point of the taxonomy.
#[derive(Debug, Clone)]
pub struct CategoryReport {
    pub category: String,
    pub scored: usize,
    pub keyword: MethodScores,
    pub vector: MethodScores,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub k: usize,
    pub overall_keyword: MethodScores,
    pub overall_vector: MethodScores,
    pub by_category: Vec<CategoryReport>,
    /// Categories with no relevant labels (e.g. `negative`): tracked, not scored in v1
    /// (true-negative precision needs a score threshold we don't have yet).
    pub unscored_categories: Vec<String>,
}

type ScoreAcc = (Vec<f64>, Vec<f64>, Vec<f64>);

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

/// Build a fresh in-memory index from the golden set, embed every document directly,
/// then score keyword and vector retrieval per query. Returns aggregated metrics.
pub async fn run_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    k: usize,
) -> Result<Report, DomainError> {
    let corpus = load_corpus();
    let queries = load_queries();

    let db = Database::open_in_memory()?;
    let repo = SqliteNoteRepository::new(db.clone());
    let keyword = SqliteKeywordIndex::new(db.clone());
    let vectors = SqliteVectorIndex::new(db.clone());

    // Map fixture id <-> stored NoteId (uuid), and insert + embed each note directly.
    let mut fixture_of: HashMap<String, String> = HashMap::new();
    for cn in &corpus {
        const DUMMY_EPOCH_MS: i64 = 1000; // arbitrary; eval only cares about content, not timestamps
        let note = Note::new(cn.title.clone(), cn.body.clone(), DUMMY_EPOCH_MS);
        let uuid = note.id.to_string();
        repo.upsert(&note).await?; // populates FTS
        let doc = format!("{}\n\n{}", cn.title, cn.body);
        let emb = embedder.embed(std::slice::from_ref(&doc)).await?;
        let emb = emb.first().ok_or_else(|| {
            DomainError::Provider("embedder returned empty batch for single doc".to_string())
        })?;
        vectors.upsert(&uuid, emb).await?;
        fixture_of.insert(uuid, cn.id.clone());
    }

    // Accumulators keyed by category, plus overall.
    let mut cat_kw: HashMap<String, ScoreAcc> = Default::default();
    let mut cat_vec: HashMap<String, ScoreAcc> = Default::default();
    let mut unscored: HashSet<String> = HashSet::new();

    for q in &queries {
        if q.relevant_ids.is_empty() {
            unscored.insert(q.category.clone());
            continue; // negatives: tracked, not scored in v1
        }
        let relevant: HashSet<String> = q.relevant_ids.iter().cloned().collect();

        let kw_ids = to_fixture(&search(&keyword, &q.query, k).await?, &fixture_of);
        let vec_ids = to_fixture(
            &vector_search(&vectors, embedder.as_ref(), &q.query, k).await?,
            &fixture_of,
        );

        push_scores(
            cat_kw.entry(q.category.clone()).or_default(),
            &kw_ids,
            &relevant,
            k,
        );
        push_scores(
            cat_vec.entry(q.category.clone()).or_default(),
            &vec_ids,
            &relevant,
            k,
        );
    }

    let mut by_category: Vec<CategoryReport> = Vec::new();
    for (cat, kw) in &cat_kw {
        let vc = cat_vec.get(cat).ok_or_else(|| {
            DomainError::Provider(format!("category {cat} missing from vector scores"))
        })?;
        by_category.push(CategoryReport {
            category: cat.clone(),
            scored: kw.0.len(),
            keyword: MethodScores {
                recall: mean(&kw.0),
                map: mean(&kw.1),
                mrr: mean(&kw.2),
            },
            vector: MethodScores {
                recall: mean(&vc.0),
                map: mean(&vc.1),
                mrr: mean(&vc.2),
            },
        });
    }
    by_category.sort_by(|a, b| a.category.cmp(&b.category));

    let overall_keyword = overall(&cat_kw);
    let overall_vector = overall(&cat_vec);
    let mut unscored_categories: Vec<String> = unscored.into_iter().collect();
    unscored_categories.sort();

    Ok(Report {
        k,
        overall_keyword,
        overall_vector,
        by_category,
        unscored_categories,
    })
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

fn push_scores(acc: &mut ScoreAcc, ranked: &[String], relevant: &HashSet<String>, k: usize) {
    if let Some(r) = recall_at_k(ranked, relevant, k) {
        acc.0.push(r);
    }
    if let Some(ap) = average_precision_at_k(ranked, relevant, k) {
        acc.1.push(ap);
    }
    if let Some(rr) = reciprocal_rank(ranked, relevant) {
        acc.2.push(rr);
    }
}

fn overall(cats: &HashMap<String, ScoreAcc>) -> MethodScores {
    let mut r = Vec::new();
    let mut m = Vec::new();
    let mut rr = Vec::new();
    for (a, b, c) in cats.values() {
        r.extend(a);
        m.extend(b);
        rr.extend(c);
    }
    MethodScores {
        recall: mean(&r),
        map: mean(&m),
        mrr: mean(&rr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use raki_ai::FakeEmbeddingProvider;
    use std::sync::Arc;

    #[tokio::test]
    async fn harness_scores_every_category_with_fake_embedder() {
        let report = run_eval(Arc::new(FakeEmbeddingProvider::new(384)), 5)
            .await
            .unwrap();
        assert_eq!(report.k, 5);
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
    }

    #[test]
    fn fixtures_parse_and_reference_real_corpus_ids() {
        let corpus = load_corpus();
        let queries = load_queries();
        assert!(corpus.len() >= 6, "need a non-trivial corpus");
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
}
