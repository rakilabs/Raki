//! Retrieval evaluation harness: load a taxonomy-tagged golden set, build a fresh
//! in-memory index, run keyword + vector retrieval, and score per category. v1 eval
//! is a bootstrap + regression tripwire, NOT a statistically-meaningful benchmark.

use std::collections::HashMap;

pub mod benchmark;
pub mod chunk;
pub mod chunk_baseline;
pub mod local_corpus;
pub mod markdown;
pub mod memory_corpus;
pub mod realmetrics;
pub use chunk_baseline::render_chunking_baseline;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CorpusNote {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalQuery {
    pub query: String,
    #[serde(default = "default_category")]
    pub category: String,
    /// "dev" (used while tuning) or "holdout" (run only by the gate). Defaults to "dev".
    #[serde(default = "default_set")]
    pub set: String,
    #[serde(default)]
    pub relevant_ids: Vec<String>,
    /// Optional graded relevance (fixture id → grade). Absent ⇒ binary; nDCG dormant.
    #[serde(default)]
    pub grades: HashMap<String, f64>,
    /// Optional single best answer (real-data tier). Mark ONLY when unambiguous. Drives
    /// Primary-Success@1; absent ⇒ excluded from that metric.
    #[serde(default)]
    pub primary: Option<String>,
}

fn default_category() -> String {
    "real".to_string()
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

/// Load the committed synthetic chunking corpus + queries (Task 5 fixtures).
pub fn load_chunking_corpus() -> Vec<CorpusNote> {
    let raw = include_str!("../fixtures/chunking/corpus.json");
    serde_json::from_str(raw).expect("chunking corpus.json parses")
}
pub fn load_chunking_queries() -> Vec<EvalQuery> {
    let raw = include_str!("../fixtures/chunking/queries.json");
    serde_json::from_str(raw).expect("chunking queries.json parses")
}

use std::collections::HashSet;
use std::sync::Arc;

use crate::chunk::{chunk, ChunkStrategy, PrefixMode, Rollup};
use async_trait::async_trait;
use raki_domain::{
    DomainError, EmbeddingProvider, Note, NoteId, NoteRepository, QueryRewriter,
    QueryUnderstanding, Reranker, VectorIndex,
};
use raki_retrieval::{
    average_precision_at_k, hybrid_candidates, hybrid_search, ndcg_at_k, recall_at_k,
    reciprocal_rank, rerank, search, vector_search,
};
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex};

/// Deterministic, no-LLM rewriter for CI-stable eval gates.
///
/// Uses naive substring matching (eval-scoped). Not suitable for production queries
/// without word-boundary guards.
pub struct RuleBasedRewriter;

#[async_trait]
impl QueryRewriter for RuleBasedRewriter {
    async fn understand(&self, query: &str) -> Result<QueryUnderstanding, DomainError> {
        let lowered = query.to_lowercase();
        let rewritten = if lowered.contains("inn") {
            lowered.replace("inn", "ryokan")
        } else if lowered.contains("spend") || lowered.contains("spent") {
            lowered
                .replace("spend", "expenses")
                .replace("spent", "expenses")
        } else {
            lowered.clone()
        };
        let changed = rewritten != lowered;
        Ok(QueryUnderstanding {
            rewritten_query: rewritten,
            needs_multi_hop: false,
            sub_queries: vec![],
            confidence: if changed { 0.8 } else { 0.0 },
            is_fallback: !changed,
        })
    }
}

/// Mean metrics for one retrieval method over a set of (scored) queries.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
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
    pub reranked: MethodScores,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub k: usize,
    pub overall_keyword: MethodScores,
    pub overall_vector: MethodScores,
    pub overall_hybrid: MethodScores,
    pub overall_reranked: MethodScores,
    pub by_category: Vec<CategoryReport>,
    /// Categories with no relevant labels (e.g. `negative`): tracked, not scored in v1
    /// (true-negative precision needs a score threshold we don't have yet).
    pub unscored_categories: Vec<String>,
}

/// One method's outcome for one query: the ranked fixture ids (top-k) and the metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodResult {
    pub ranked: Vec<String>, // fixture ids, best-first, truncated to k
    pub scores: MethodScores,
}

/// Per-query detail for every method — the substrate for the audit (3a-i) and the
/// per-query snapshot gate (3a-ii).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub query: String,
    pub category: String,
    pub set: String,
    pub keyword: MethodResult,
    pub vector: MethodResult,
    pub hybrid: MethodResult,
    pub reranked: MethodResult,
}

/// Which retrieval method a snapshot check targets. The deterministic gate checks
/// `Keyword` only (model-independent); the real-model gate checks all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Keyword,
    Vector,
    Hybrid,
    Reranked,
}

impl QueryResult {
    pub fn method(&self, m: Method) -> &MethodResult {
        match m {
            Method::Keyword => &self.keyword,
            Method::Vector => &self.vector,
            Method::Hybrid => &self.hybrid,
            Method::Reranked => &self.reranked,
        }
    }
}

/// FNV-1a 64-bit over the embedded fixture text — a deterministic change-detector
/// (house style, see raki-storage/src/hash.rs; not a security hash). Lets a reviewer
/// confirm a committed baseline matches the fixtures it was generated from.
pub fn fixtures_fingerprint() -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in CORPUS_JSON.bytes().chain(QUERIES_JSON.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Per-metric float tolerance — these metrics are deterministic functions of rank
/// positions, so any real drop exceeds this; the epsilon only absorbs float noise.
const METRIC_EPS: f64 = 1e-9;

/// Compare a fresh run's per-query results against a committed baseline snapshot.
/// Returns one human-readable message per regression; empty ⇒ no regression.
/// A gated metric (recall@3, MAP@3, MRR, and — where both runs have them — nDCG@3 and
/// recall@10) dropping below baseline is a regression. A query present in one run but
/// not the other demands an explicit re-baseline. `methods` selects which retrieval
/// methods to check: `[Keyword]` for the deterministic gate, all three for real-model.
pub fn snapshot_regressions(
    current: &[QueryResult],
    baseline: &[QueryResult],
    methods: &[Method],
) -> Vec<String> {
    let mut base: HashMap<&str, &QueryResult> =
        baseline.iter().map(|q| (q.query.as_str(), q)).collect();

    let mut out = Vec::new();
    for c in current {
        let Some(b) = base.remove(c.query.as_str()) else {
            out.push(format!(
                "query {:?} not in baseline (new query — re-baseline)",
                c.query
            ));
            continue;
        };
        for &m in methods {
            let (cm, bm) = (c.method(m), b.method(m));
            let mut check_drop = |name: &str, cv: f64, bv: f64| {
                if cv + METRIC_EPS < bv {
                    out.push(format!(
                        "{:?} [{:?}] {name} {cv:.4} < baseline {bv:.4}",
                        c.query, m
                    ));
                }
            };
            check_drop("recall@3", cm.scores.recall, bm.scores.recall);
            check_drop("MAP@3", cm.scores.map, bm.scores.map);
            check_drop("MRR", cm.scores.mrr, bm.scores.mrr);
            if let (Some(cv), Some(bv)) = (cm.scores.ndcg, bm.scores.ndcg) {
                check_drop("nDCG@3", cv, bv);
            }
            if let (Some(cv), Some(bv)) = (cm.scores.recall_cov, bm.scores.recall_cov) {
                check_drop("recall@10", cv, bv);
            }
        }
    }
    for (_, b) in base {
        out.push(format!(
            "query {:?} in baseline but absent from current run (re-baseline)",
            b.query
        ));
    }
    out
}

/// Repo-root `docs/eval` dir, relative to this crate (src-tauri/crates/raki-eval).
pub fn eval_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/eval")
}

/// Path to the committed per-query baseline, relative to this crate. `raki-eval` lives at
/// `src-tauri/crates/raki-eval`, so the repo root is three parents up.
pub fn snapshot_path() -> std::path::PathBuf {
    eval_dir().join("snapshot.json")
}

/// Load the committed baseline snapshot. Returns an error if the file is missing or
/// malformed — the gate cannot run without a committed baseline (generate it with
/// `eval-report --write`).
pub fn load_snapshot() -> Result<Vec<QueryResult>, Box<dyn std::error::Error>> {
    let path = snapshot_path();
    let text = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "read snapshot {}: {e} — run `eval-report --write` first",
            path.display()
        )
    })?;
    Ok(serde_json::from_str(&text)?)
}

/// Everything one eval run produces.
#[derive(Debug, Clone)]
pub struct EvalRun {
    pub report: Report,
    pub per_query: Vec<QueryResult>,
}

const COVERAGE_K: usize = 10;

/// Candidate depth the reranker reorders — the recall union pulled before rerank. Mirrors
/// `raki_retrieval::HYBRID_CANDIDATE_POOL`; production may later use a latency-bounded window.
const RERANK_POOL: usize = 20;

/// Construct a fresh, empty in-memory index stack (relational + FTS5 + vectors) in one SQLite
/// file. The single source of truth for index construction — both `run_eval_over` (synthetic
/// fixtures) and `run_benchmark` (BEIR) build on it (AGENTS.md §5/§9). Callers do their own
/// corpus-loading loop (chunked vs whole-doc differ).
pub fn build_in_memory_index() -> Result<
    (
        Database,
        SqliteNoteRepository,
        SqliteKeywordIndex,
        SqliteVectorIndex,
    ),
    DomainError,
> {
    let db = Database::open_in_memory()?;
    let repo = SqliteNoteRepository::new(db.clone());
    let keyword = SqliteKeywordIndex::new(db.clone());
    let vectors = SqliteVectorIndex::new(db.clone());
    Ok((db, repo, keyword, vectors))
}

/// Build a fresh in-memory index from the golden set, embed every document directly,
/// then score keyword and vector retrieval per query. Returns aggregated metrics
/// plus per-query detail.
/// Run the eval over a CALLER-SUPPLIED corpus + queries (the synthetic fixtures, or a local
/// real-notes set). All retrieval/scoring logic lives here; `run_eval` is the fixture wrapper.
#[allow(clippy::too_many_arguments)]
pub async fn run_eval_over(
    corpus: &[CorpusNote],
    queries: &[EvalQuery],
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
    strategy: ChunkStrategy,
    prefix: PrefixMode,
    rollup: Rollup,
    rewriter: Option<&dyn QueryRewriter>,
) -> Result<EvalRun, DomainError> {
    let (_db, repo, keyword, vectors) = build_in_memory_index()?;

    // text_of is keyed by BOTH chunk id and note uuid → the text to (re)rank for that id.
    // fixture_of maps both chunk ids and the note uuid to the fixture slug.
    let mut fixture_of: HashMap<String, String> = HashMap::new();
    let mut text_of: HashMap<String, String> = HashMap::new();
    for (idx, cn) in corpus.iter().enumerate() {
        const DUMMY_EPOCH_MS: i64 = 1000;
        let mut note = Note::new(cn.title.clone(), cn.body.clone(), DUMMY_EPOCH_MS);
        // Deterministic note id derived from the corpus position, so the keyword
        // tie-break (`ORDER BY score, note_id`, 3a-i) is stable across runs and machines.
        // `Note::new` would mint a random UUIDv7, which makes the snapshot's tie ordering
        // non-reproducible — vacuous determinism that 3b's near-duplicate clusters (where
        // bm25 ties are likely) would expose as flaky CI. A stable id makes the guarantee real.
        note.id = NoteId::parse(&format!("00000000-0000-7000-8000-{:012x}", idx + 1))
            .expect("synthetic fixture uuid is well-formed");
        let uuid = note.id.to_string();
        repo.upsert(&note).await?; // keyword/FTS5 over the WHOLE note — unchanged, gate-safe.

        // Note-level entries: keyword hits return the uuid; keyword-backfilled rerank candidates
        // need the whole-note text.
        let whole = format!("{}\n\n{}", cn.title, cn.body);
        fixture_of.insert(uuid.clone(), cn.id.clone());
        text_of.insert(uuid.clone(), whole.clone());

        let texts = chunk(&cn.title, &cn.body, strategy, prefix);
        for (j, text) in texts.iter().enumerate() {
            // WholeNote (single chunk): id == note uuid, so the vector keying is byte-identical to
            // the legacy path. Blocks: `uuid#j`.
            let chunk_id = if strategy == ChunkStrategy::WholeNote {
                uuid.clone()
            } else {
                format!("{uuid}#{j}")
            };
            let emb = embedder.embed(std::slice::from_ref(text)).await?;
            let emb = emb.first().ok_or_else(|| {
                DomainError::Provider("embedder returned empty batch".to_string())
            })?;
            vectors.upsert(&chunk_id, emb).await?;
            fixture_of.insert(chunk_id.clone(), cn.id.clone());
            text_of.insert(chunk_id, text.clone());
        }
    }

    let mut per_query: Vec<QueryResult> = Vec::new();
    let mut unscored: HashSet<String> = HashSet::new();

    for q in queries {
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

        let kw = dedup_to_note(&to_fixture(
            &search(&keyword, &q.query, cov_k.max(k)).await?,
            &fixture_of,
        ));
        let vc = match rollup {
            Rollup::MinRank => dedup_to_note(&to_fixture(
                &vector_search(&vectors, embedder.as_ref(), &q.query, cov_k.max(k)).await?,
                &fixture_of,
            )),
            Rollup::ScoreMax => {
                // Embed the query only here — MinRank re-embeds inside vector_search.
                let q_emb = embedder.embed(std::slice::from_ref(&q.query)).await?;
                let q_emb = q_emb.into_iter().next().ok_or_else(|| {
                    DomainError::Provider("embedder returned empty batch for query".to_string())
                })?;
                let hits = vectors.query(&q_emb, RERANK_POOL).await?;
                let scored: Vec<(String, f32)> = hits
                    .into_iter()
                    .map(|h| (h.source_id, h.distance))
                    .collect();
                score_max_notes(&scored, &fixture_of)
            }
        };
        let hy = dedup_to_note(&to_fixture(
            &hybrid_search(
                &keyword,
                &vectors,
                embedder.as_ref(),
                rewriter,
                &q.query,
                cov_k.max(k),
            )
            .await?
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>(),
            &fixture_of,
        ));
        let raw_pool = hybrid_candidates(
            &keyword,
            &vectors,
            embedder.as_ref(),
            rewriter,
            &q.query,
            RERANK_POOL,
        )
        .await?;
        let candidates: Vec<(String, String)> = raw_pool
            .iter()
            .map(|id| id.to_string())
            .filter_map(|id_str| text_of.get(&id_str).map(|t| (id_str, t.clone())))
            .collect();
        let rr = match rollup {
            Rollup::MinRank => dedup_to_note(&to_fixture(
                &rerank(reranker.as_ref(), &q.query, &candidates, RERANK_POOL).await?,
                &fixture_of,
            )),
            Rollup::ScoreMax => {
                let scores = reranker
                    .rerank(
                        &q.query,
                        &candidates
                            .iter()
                            .map(|(_, t)| t.clone())
                            .collect::<Vec<_>>(),
                    )
                    .await?;
                // higher score = better; convert to a "distance" (negate) so score_max_notes' min works.
                let as_dist: Vec<(String, f32)> = scores
                    .into_iter()
                    .map(|s| (candidates[s.index].0.clone(), -s.score))
                    .collect();
                score_max_notes(&as_dist, &fixture_of)
            }
        };

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
            reranked: MethodResult {
                scores: score_one(&rr, &relevant, k, q),
                ranked: truncate(&rr, k),
            },
        });
    }

    let report = aggregate(&per_query, &mut unscored, k);
    Ok(EvalRun { report, per_query })
}

/// Eval over the committed synthetic fixtures (the smoke/regression tier).
pub async fn run_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    run_eval_over(
        &load_corpus(),
        &load_queries(),
        embedder,
        reranker,
        k,
        ChunkStrategy::WholeNote,
        PrefixMode::Title,
        Rollup::MinRank,
        None,
    )
    .await
}

/// Eval over the committed synthetic fixtures WITH the rule-based rewriter.
pub async fn run_eval_with_rewrite(
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    run_eval_over(
        &load_corpus(),
        &load_queries(),
        embedder,
        reranker,
        k,
        ChunkStrategy::WholeNote,
        PrefixMode::Title,
        Rollup::MinRank,
        Some(&RuleBasedRewriter),
    )
    .await
}

fn truncate(ids: &[String], k: usize) -> Vec<String> {
    ids.iter().take(k).cloned().collect()
}

fn score_one(
    ranked: &[String],
    relevant: &HashSet<String>,
    k: usize,
    q: &EvalQuery,
) -> MethodScores {
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
            reranked: mean_scores(qrs.iter().map(|q| q.reranked.scores)),
        });
    }
    let overall_keyword = mean_scores(per_query.iter().map(|q| q.keyword.scores));
    let overall_vector = mean_scores(per_query.iter().map(|q| q.vector.scores));
    let overall_hybrid = mean_scores(per_query.iter().map(|q| q.hybrid.scores));
    let overall_reranked = mean_scores(per_query.iter().map(|q| q.reranked.scores));
    let mut unscored_categories: Vec<String> = unscored.drain().collect();
    unscored_categories.sort();
    Report {
        k,
        overall_keyword,
        overall_vector,
        overall_hybrid,
        overall_reranked,
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

/// Roll chunk hits up to a note ranking by SCORE-MAX: each note's score is its best (lowest-
/// distance) chunk; notes are ordered best-first. Distinct from min-rank when a note's best chunk
/// is not its first-appearing chunk (after rerank, or with quantization noise) — see spec D4.
fn score_max_notes(
    hits: &[(String, f32)],
    fixture_of: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    use std::collections::HashMap;
    let mut best: HashMap<String, f32> = HashMap::new();
    for (chunk_id, dist) in hits {
        if let Some(slug) = fixture_of.get(chunk_id) {
            best.entry(slug.clone())
                .and_modify(|d| {
                    if *dist < *d {
                        *d = *dist
                    }
                })
                .or_insert(*dist);
        }
    }
    let mut notes: Vec<(String, f32)> = best.into_iter().collect();
    notes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    notes.into_iter().map(|(slug, _)| slug).collect()
}

/// Roll a chunk-level slug list up to a note ranking by MIN-RANK: a note's position is its
/// first (best-ranked) chunk. A no-op when each note has one chunk (WholeNote). NOT score-max
/// (a note's best-*scored* chunk) — see `score_max_notes` and the spec D4.
fn dedup_to_note(slugs: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for s in slugs {
        if seen.insert(s.clone()) {
            out.push(s.clone());
        }
    }
    out
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

    use raki_ai::{FakeEmbeddingProvider, FakeReranker};
    use std::sync::Arc;

    #[tokio::test]
    async fn rule_based_rewriter_expands_inn() {
        let rw = RuleBasedRewriter;
        let u = rw.understand("how do I pay at the inn?").await.unwrap();
        assert!(u.rewritten_query.contains("ryokan"));
        assert!(!u.is_fallback);
    }

    #[tokio::test]
    async fn rule_based_rewriter_expands_spend() {
        let rw = RuleBasedRewriter;
        let u = rw.understand("how much did we spend?").await.unwrap();
        assert!(u.rewritten_query.contains("expenses"));
        assert!(!u.is_fallback);
    }

    #[tokio::test]
    async fn rule_based_rewriter_expands_spent() {
        let rw = RuleBasedRewriter;
        let u = rw.understand("what I spent in Kyoto").await.unwrap();
        assert!(u.rewritten_query.contains("expenses"));
        assert!(!u.is_fallback);
    }

    #[tokio::test]
    async fn rule_based_rewriter_fallback_on_unmatched() {
        let rw = RuleBasedRewriter;
        let u = rw.understand("hello world").await.unwrap();
        assert_eq!(u.rewritten_query, "hello world");
        assert!(u.is_fallback);
        assert_eq!(u.confidence, 0.0);
    }

    #[tokio::test]
    async fn harness_scores_every_category_with_fake_embedder() {
        let run = run_eval(
            Arc::new(FakeEmbeddingProvider::new(384)),
            Arc::new(FakeReranker),
            5,
        )
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
        // reranked is computed, in range, for every scored category, and carries nDCG on
        // graded categories (the metric it is meant to move).
        for c in &report.by_category {
            assert!(c.reranked.recall >= 0.0 && c.reranked.recall <= 1.0);
        }
        assert!(report.overall_reranked.recall >= 0.0 && report.overall_reranked.recall <= 1.0);
        for cat in ["dense-near-duplicate", "paraphrase-distractor"] {
            let q = run
                .per_query
                .iter()
                .find(|q| q.category == cat)
                .unwrap_or_else(|| panic!("missing {cat}"));
            assert!(
                q.reranked.scores.ndcg.is_some(),
                "{cat} reranked must carry nDCG (graded)"
            );
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
        for cat in ["dense-near-duplicate", "paraphrase-distractor"] {
            let q = run
                .per_query
                .iter()
                .find(|q| q.category == cat)
                .unwrap_or_else(|| panic!("missing {cat}"));
            assert!(
                q.keyword.scores.ndcg.is_some(),
                "{cat} must produce nDCG (graded)"
            );
        }
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
        assert!(corpus.len() >= 28, "need a non-trivial corpus");
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
    fn new_failure_mode_categories_present() {
        let queries = load_queries();
        for c in ["dense-near-duplicate", "paraphrase-distractor", "polysemy"] {
            assert!(
                queries.iter().any(|q| q.category == c),
                "missing mandatory 3b category {c}"
            );
        }
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
    fn eval_query_parses_optional_primary_and_default_category() {
        // category omitted → defaults; primary present.
        let q: EvalQuery =
            serde_json::from_str(r#"{ "query": "q", "relevant_ids": ["a","b"], "primary": "a" }"#)
                .unwrap();
        assert_eq!(q.category, "real");
        assert_eq!(q.primary.as_deref(), Some("a"));
        // primary omitted → None.
        let q2: EvalQuery = serde_json::from_str(
            r#"{ "query": "q2", "category": "exact", "relevant_ids": ["c"] }"#,
        )
        .unwrap();
        assert_eq!(q2.category, "exact");
        assert_eq!(q2.primary, None);
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

    fn mk(query: &str, category: &str, kw_recall: f64, ndcg: Option<f64>) -> QueryResult {
        let scores = MethodScores {
            recall: kw_recall,
            map: 1.0,
            mrr: 1.0,
            ndcg,
            recall_cov: None,
        };
        let mr = MethodResult {
            ranked: vec!["n1".into()],
            scores,
        };
        QueryResult {
            query: query.into(),
            category: category.into(),
            set: "dev".into(),
            keyword: mr.clone(),
            vector: mr.clone(),
            hybrid: mr.clone(),
            reranked: mr,
        }
    }

    #[test]
    fn identical_runs_have_no_regression() {
        let base = vec![mk("q1", "lexical-overlap", 1.0, None)];
        let cur = base.clone();
        assert!(snapshot_regressions(&cur, &base, &[Method::Keyword]).is_empty());
    }

    #[test]
    fn a_metric_drop_is_a_regression() {
        let base = vec![mk("q1", "lexical-overlap", 1.0, None)];
        let cur = vec![mk("q1", "lexical-overlap", 0.5, None)];
        let r = snapshot_regressions(&cur, &base, &[Method::Keyword]);
        assert_eq!(r.len(), 1, "recall drop must be reported once");
        assert!(r[0].contains("recall@3"));
    }

    #[test]
    fn an_ndcg_drop_on_ordering_is_a_regression() {
        let base = vec![mk("E0599", "lexical-cluster", 1.0, Some(0.92))];
        let cur = vec![mk("E0599", "lexical-cluster", 1.0, Some(0.73))];
        let r = snapshot_regressions(&cur, &base, &[Method::Keyword]);
        assert!(
            r.iter().any(|m| m.contains("nDCG@3")),
            "demoted direct answer must trip nDCG"
        );
    }

    #[test]
    fn an_improvement_is_not_a_regression() {
        let base = vec![mk("q1", "lexical-overlap", 0.5, None)];
        let cur = vec![mk("q1", "lexical-overlap", 1.0, None)];
        assert!(snapshot_regressions(&cur, &base, &[Method::Keyword]).is_empty());
    }

    #[test]
    fn a_missing_or_new_query_demands_rebaseline() {
        let base = vec![mk("q1", "lexical-overlap", 1.0, None)];
        let cur = vec![mk("q2", "lexical-overlap", 1.0, None)];
        let r = snapshot_regressions(&cur, &base, &[Method::Keyword]);
        assert_eq!(r.len(), 2, "q1 absent + q2 new ⇒ two re-baseline messages");
        assert!(r.iter().any(|m| m.contains("absent")));
        assert!(r.iter().any(|m| m.contains("not in baseline")));
    }

    #[test]
    fn dedup_to_note_keeps_first_occurrence_order() {
        // chunk hits map to slugs with repeats; min-rank = first occurrence per note.
        let slugs = vec!["a".into(), "a".into(), "b".into(), "a".into(), "c".into()];
        assert_eq!(
            dedup_to_note(&slugs),
            vec!["a".to_string(), "b".into(), "c".into()]
        );
        assert_eq!(dedup_to_note(&[]), Vec::<String>::new());
    }

    #[test]
    fn score_max_orders_notes_by_best_chunk_and_can_differ_from_min_rank() {
        // chunk hits (id, distance) — lower distance = better. fixture maps chunks→notes.
        let mut fx = std::collections::HashMap::new();
        fx.insert("X#0".to_string(), "X".to_string());
        fx.insert("X#1".to_string(), "X".to_string());
        fx.insert("Y#0".to_string(), "Y".to_string());
        // rank order (by distance): X#0(0.30), Y#0(0.31), X#1(0.10)
        let _hits = [
            (("X#0").to_string(), 0.30_f32),
            (("Y#0").to_string(), 0.31),
            (("X#1").to_string(), 0.10),
        ];
        // min-rank would be [X, Y] (X first at rank 1). score-max sees X's best is 0.10 < Y's 0.31,
        // and Y's best 0.31 — so X then Y; here they agree on X, but the BEST score for X is X#1
        // not X#0, which min-rank could never surface. Construct divergence with a third note:
        fx.insert("Z#0".to_string(), "Z".to_string());
        let hits2 = vec![
            (("Z#0").to_string(), 0.20_f32), // Z best 0.20, rank 1
            (("X#0").to_string(), 0.25),     // X first appears rank 2
            (("X#1").to_string(), 0.05),     // X best 0.05 (better than Z) but rank 3
        ];
        // min-rank: [Z, X]; score-max: [X, Z] (X's best 0.05 beats Z's 0.20).
        assert_eq!(
            score_max_notes(&hits2, &fx),
            vec!["X".to_string(), "Z".into()]
        );
    }
}
