//! BEIR benchmark measurement tier (manual). Loads a downloaded public IR dataset (SciFact),
//! scores the four production retrieval methods in aggregate (nDCG@10 / Recall@10 / MAP), and
//! reports the `reranked − hybrid` delta + a vector-sanity number. Not a regression gate; not a
//! production change. See the R0 spec + ADR-0007.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use raki_domain::{
    DomainError, EmbeddingProvider, Note, NoteId, NoteRepository, Reranker, VectorIndex,
};
use raki_retrieval::{
    average_precision_at_k, hybrid_candidates, hybrid_search, ndcg_at_k, rerank, search,
    vector_search,
};

use crate::build_in_memory_index;

/// One BEIR corpus document.
#[derive(Debug, Clone, PartialEq)]
pub struct BeirDoc {
    pub id: String,
    pub title: String,
    pub text: String,
}

/// A parsed BEIR dataset: corpus, test queries `(qid, text)`, and graded qrels
/// `qid → { doc_id → grade }`.
#[derive(Debug, Default)]
pub struct BeirData {
    pub corpus: Vec<BeirDoc>,
    pub queries: Vec<(String, String)>,
    pub qrels: HashMap<String, HashMap<String, f64>>,
}

/// Parse `corpus.jsonl` (`{"_id","title","text"}` per line; blank lines skipped).
pub fn parse_corpus(jsonl: &str) -> Result<Vec<BeirDoc>, String> {
    jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| format!("corpus line: {e}"))?;
            Ok(BeirDoc {
                id: v["_id"].as_str().ok_or("corpus: missing _id")?.to_string(),
                title: v["title"].as_str().unwrap_or("").to_string(),
                text: v["text"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Parse `queries.jsonl` (`{"_id","text"}` per line) into `(qid, text)`.
pub fn parse_queries(jsonl: &str) -> Result<Vec<(String, String)>, String> {
    jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: serde_json::Value =
                serde_json::from_str(line).map_err(|e| format!("query line: {e}"))?;
            Ok((
                v["_id"].as_str().ok_or("query: missing _id")?.to_string(),
                v["text"].as_str().unwrap_or("").to_string(),
            ))
        })
        .collect()
}

/// Parse `qrels/test.tsv` (`query-id<TAB>corpus-id<TAB>score`, with a header row skipped).
pub fn parse_qrels(tsv: &str) -> Result<HashMap<String, HashMap<String, f64>>, String> {
    let mut out: HashMap<String, HashMap<String, f64>> = HashMap::new();
    for line in tsv.lines().filter(|l| !l.trim().is_empty()) {
        let mut cols = line.split('\t');
        let (Some(qid), Some(did), Some(score)) = (cols.next(), cols.next(), cols.next()) else {
            return Err(format!("qrels: malformed line: {line:?}"));
        };
        // Skip the header row.
        if qid == "query-id" {
            continue;
        }
        let grade: f64 = score
            .trim()
            .parse()
            .map_err(|e| format!("qrels: bad score {score:?}: {e}"))?;
        out.entry(qid.to_string())
            .or_default()
            .insert(did.to_string(), grade);
    }
    Ok(out)
}

const SCIFACT_URL: &str =
    "https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/scifact.zip";

/// A dataset dir is valid iff all three files exist and their first non-empty line parses.
pub fn validate_dataset(dir: &Path) -> Result<(), String> {
    let corpus = std::fs::read_to_string(dir.join("corpus.jsonl"))
        .map_err(|e| format!("corpus.jsonl: {e}"))?;
    parse_corpus(
        corpus
            .lines()
            .take(1)
            .collect::<Vec<_>>()
            .join("\n")
            .as_str(),
    )?;
    let queries = std::fs::read_to_string(dir.join("queries.jsonl"))
        .map_err(|e| format!("queries.jsonl: {e}"))?;
    parse_queries(
        queries
            .lines()
            .take(1)
            .collect::<Vec<_>>()
            .join("\n")
            .as_str(),
    )?;
    let qrels = std::fs::read_to_string(dir.join("qrels/test.tsv"))
        .map_err(|e| format!("qrels/test.tsv: {e}"))?;
    parse_qrels(&qrels)?;
    Ok(())
}

/// Ensure a validated SciFact dataset under `cache_root/scifact`, downloading from `url` if
/// absent. Reuses the cache only if it re-validates; a corrupt cache is deleted and surfaced.
/// Download lands in a temp dir, is validated, then atomically renamed — an interrupted run
/// never leaves a half-populated canonical dir.
pub fn ensure_scifact_in(cache_root: &Path, url: &str) -> Result<PathBuf, String> {
    let final_dir = cache_root.join("scifact");
    if final_dir.exists() {
        return match validate_dataset(&final_dir) {
            Ok(()) => Ok(final_dir),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&final_dir);
                Err(format!(
                    "corrupt cache at {final_dir:?} ({e}); deleted — rerun to re-download"
                ))
            }
        };
    }
    let tmp = cache_root.join(format!("scifact.tmp.{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("mkdir tmp: {e}"))?;
    let unpacked = download_and_unzip(url, &tmp).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&tmp);
    })?;
    if let Err(e) = validate_dataset(&unpacked) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("downloaded dataset failed validation: {e}"));
    }
    std::fs::create_dir_all(cache_root).ok();
    std::fs::rename(&unpacked, &final_dir).map_err(|e| format!("promote: {e}"))?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(final_dir)
}

/// Public entry: cache under `<repo>/.beir_cache`.
pub fn ensure_scifact() -> Result<PathBuf, String> {
    ensure_scifact_in(Path::new(".beir_cache"), SCIFACT_URL)
}

/// Download `url` and unzip into `dest`, returning the `scifact/` subdir the BEIR zip nests under.
fn download_and_unzip(url: &str, dest: &Path) -> Result<PathBuf, String> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("download {url}: {e}"))?;
    let bytes = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("read body: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("open zip: {e}"))?;
    zip.extract(dest).map_err(|e| format!("unzip: {e}"))?;
    Ok(dest.join("scifact"))
}

/// Mean aggregate for one retrieval method over the scored queries.
#[derive(Debug, Clone, Copy)]
pub struct MethodAgg {
    pub ndcg: f64,
    pub recall: f64,
    pub map: f64,
}

#[derive(Debug, Clone)]
pub struct BenchReport {
    pub queries_scored: usize,
    pub keyword: MethodAgg,
    pub vector: MethodAgg,
    pub hybrid: MethodAgg,
    pub reranked: MethodAgg,
    /// The headline R1 signal: reranked nDCG@10 − hybrid nDCG@10.
    pub reranked_minus_hybrid_ndcg: f64,
}

/// Recall-union depth the reranker reorders (IR convention; tunable for speed).
const RERANK_POOL: usize = 100;

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

/// Score the four production retrieval methods over a BEIR dataset, in aggregate (nDCG@10 /
/// Recall@10 / MAP meaned over queries that have ≥1 relevant doc; zero-relevance queries are
/// skipped — the primitives return `None`, never NaN).
pub async fn run_benchmark(
    data: &BeirData,
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<BenchReport, DomainError> {
    let (_db, repo, keyword, vectors) = build_in_memory_index()?;

    // Index every doc with a deterministic synthetic uuid; keep uuid → beir-id and uuid → text.
    let mut id_of: HashMap<String, String> = HashMap::new();
    let mut text_of: HashMap<String, String> = HashMap::new();
    let mut uuids: Vec<String> = Vec::with_capacity(data.corpus.len());
    let mut texts: Vec<String> = Vec::with_capacity(data.corpus.len());
    for (idx, d) in data.corpus.iter().enumerate() {
        let mut note = Note::new(d.title.clone(), d.text.clone(), 1000);
        note.id = NoteId::parse(&format!("00000000-0000-7000-8000-{:012x}", idx + 1))
            .expect("synthetic uuid is well-formed");
        let uuid = note.id.to_string();
        repo.upsert(&note).await?; // relational + FTS5
        let text = format!("{}\n\n{}", d.title, d.text);
        id_of.insert(uuid.clone(), d.id.clone());
        text_of.insert(uuid.clone(), text.clone());
        uuids.push(uuid);
        texts.push(text);
    }
    // Batch embed all docs in one call — ONNX overhead per-call dominates on CPU.
    let embs = embedder.embed(&texts).await?;
    if embs.len() != uuids.len() {
        return Err(DomainError::Provider(format!(
            "embedder returned {} embeddings for {} docs",
            embs.len(),
            uuids.len()
        )));
    }
    for (uuid, emb) in uuids.iter().zip(embs.iter()) {
        vectors.upsert(uuid, emb).await?;
    }
    let to_beir = |uuids: Vec<String>| -> Vec<String> {
        uuids.iter().filter_map(|u| id_of.get(u).cloned()).collect()
    };

    // Per-method accumulators of per-query Some(metric) values.
    let (mut kw, mut vc, mut hy, mut rr) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut scored = 0usize;

    for (qid, qtext) in &data.queries {
        let grades = match data.qrels.get(qid) {
            Some(g) if g.values().any(|&s| s > 0.0) => g,
            _ => continue, // zero-relevance → skip (no relevant docs to score against)
        };
        let relevant: std::collections::HashSet<String> = grades
            .iter()
            .filter(|(_, &s)| s > 0.0)
            .map(|(d, _)| d.clone())
            .collect();
        scored += 1;

        let kw_ids = to_beir(search(&keyword, qtext, k).await?);
        let vc_ids = to_beir(vector_search(&vectors, embedder.as_ref(), qtext, k).await?);
        let hy_ids = to_beir(hybrid_search(&keyword, &vectors, embedder.as_ref(), qtext, k).await?);
        // Rerank the recall union, then map to beir ids.
        let pool =
            hybrid_candidates(&keyword, &vectors, embedder.as_ref(), qtext, RERANK_POOL).await?;
        let candidates: Vec<(String, String)> = pool
            .iter()
            .filter_map(|u| text_of.get(u).map(|t| (u.clone(), t.clone())))
            .collect();
        let rr_ids = to_beir(rerank(reranker.as_ref(), qtext, &candidates, k).await?);

        // nDCG uses graded gains; recall/MAP use the binary relevant set.
        for (acc, ids) in [
            (&mut kw, &kw_ids),
            (&mut vc, &vc_ids),
            (&mut hy, &hy_ids),
            (&mut rr, &rr_ids),
        ] {
            let ndcg = ndcg_at_k(ids, grades, k);
            let recall = raki_retrieval::recall_at_k(ids, &relevant, k);
            let map = average_precision_at_k(ids, &relevant, k);
            // All three are Some here (relevant non-empty); push for the mean.
            if let (Some(n), Some(r), Some(m)) = (ndcg, recall, map) {
                acc.push((n, r, m));
            }
        }
    }

    let agg = |v: &[(f64, f64, f64)]| MethodAgg {
        ndcg: mean(&v.iter().map(|t| t.0).collect::<Vec<_>>()),
        recall: mean(&v.iter().map(|t| t.1).collect::<Vec<_>>()),
        map: mean(&v.iter().map(|t| t.2).collect::<Vec<_>>()),
    };
    let (keyword, vector, hybrid, reranked) = (agg(&kw), agg(&vc), agg(&hy), agg(&rr));
    Ok(BenchReport {
        queries_scored: scored,
        reranked_minus_hybrid_ndcg: reranked.ndcg - hybrid.ndcg,
        keyword,
        vector,
        hybrid,
        reranked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_corpus_queries_qrels() {
        let corpus = parse_corpus(
            "{\"_id\":\"d1\",\"title\":\"T\",\"text\":\"body one\"}\n\n{\"_id\":\"d2\",\"title\":\"\",\"text\":\"two\"}\n",
        )
        .unwrap();
        assert_eq!(corpus.len(), 2);
        assert_eq!(
            corpus[0],
            BeirDoc {
                id: "d1".into(),
                title: "T".into(),
                text: "body one".into()
            }
        );

        let queries = parse_queries("{\"_id\":\"q1\",\"text\":\"a claim\"}\n").unwrap();
        assert_eq!(queries, vec![("q1".to_string(), "a claim".to_string())]);

        let qrels = parse_qrels("query-id\tcorpus-id\tscore\nq1\td1\t1\nq1\td2\t2\n").unwrap();
        assert_eq!(qrels["q1"]["d1"], 1.0);
        assert_eq!(qrels["q1"]["d2"], 2.0);
        assert_eq!(qrels["q1"].len(), 2, "header row skipped");
    }

    use std::fs;

    #[test]
    fn validate_rejects_incomplete_or_corrupt_dir() {
        let base = std::env::temp_dir().join(format!("raki_bench_t_{}", std::process::id()));
        let dir = base.join("scifact");
        fs::create_dir_all(dir.join("qrels")).unwrap();
        // Missing all files → invalid.
        assert!(validate_dataset(&dir).is_err(), "empty dir invalid");
        // Present but unparseable corpus → invalid.
        fs::write(dir.join("corpus.jsonl"), "not json").unwrap();
        fs::write(dir.join("queries.jsonl"), "{\"_id\":\"q\",\"text\":\"x\"}").unwrap();
        fs::write(
            dir.join("qrels/test.tsv"),
            "query-id\tcorpus-id\tscore\nq\td\t1\n",
        )
        .unwrap();
        assert!(validate_dataset(&dir).is_err(), "bad corpus jsonl invalid");
        // Fix corpus → valid.
        fs::write(
            dir.join("corpus.jsonl"),
            "{\"_id\":\"d\",\"title\":\"\",\"text\":\"x\"}",
        )
        .unwrap();
        assert!(validate_dataset(&dir).is_ok(), "now valid");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_deletes_a_corrupt_cache_and_errors_offline() {
        // A pre-existing but corrupt final dir must be removed and surfaced as an error
        // (no network attempted here — invalid URL would only be hit on re-download).
        let root = std::env::temp_dir().join(format!("raki_bench_c_{}", std::process::id()));
        let final_dir = root.join("scifact");
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("corpus.jsonl"), "garbage").unwrap();
        let err = ensure_scifact_in(&root, "http://invalid.invalid/none.zip").unwrap_err();
        assert!(err.contains("corrupt"), "got: {err}");
        assert!(!final_dir.exists(), "corrupt cache deleted");
        let _ = fs::remove_dir_all(&root);
    }

    use raki_ai::{FakeEmbeddingProvider, FakeReranker};

    #[tokio::test]
    async fn run_benchmark_aggregates_and_skips_zero_relevance() {
        let data = BeirData {
            corpus: vec![
                BeirDoc {
                    id: "d1".into(),
                    title: "apples".into(),
                    text: "granny smith apples".into(),
                },
                BeirDoc {
                    id: "d2".into(),
                    title: "oranges".into(),
                    text: "navel oranges".into(),
                },
            ],
            queries: vec![
                ("q1".into(), "apples".into()),  // relevant: d1
                ("qz".into(), "nothing".into()), // zero-relevance → skipped
            ],
            qrels: HashMap::from([
                ("q1".to_string(), HashMap::from([("d1".to_string(), 1.0)])),
                ("qz".to_string(), HashMap::new()),
            ]),
        };
        let embedder = Arc::new(FakeEmbeddingProvider::new(384));
        let reranker = Arc::new(FakeReranker);
        let rep = run_benchmark(&data, embedder, reranker, 10).await.unwrap();
        assert_eq!(rep.queries_scored, 1, "zero-relevance query skipped");
        // Keyword must find the lexical match for the scored query.
        assert!(rep.keyword.recall > 0.0, "keyword finds d1 for 'apples'");
        assert!(rep.vector.ndcg >= 0.0 && rep.vector.ndcg <= 1.0);
        assert!(rep.reranked_minus_hybrid_ndcg.is_finite());
    }
}
