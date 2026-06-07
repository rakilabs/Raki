//! `real-eval`: LOCAL-ONLY measurement on real Markdown notes + labeled queries under
//! `eval-data/real/` (gitignored). Prints per-method / per-query / per-category detail to the
//! terminal (never written to git) and writes ONLY a content-free aggregate baseline.
//! Directional signal — not statistically powered. See the real-data spec, Limitations.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_domain::{EmbeddingProvider, Reranker};
use raki_eval::local_corpus::load_local;
use raki_eval::realmetrics::{primary_success_at_1, success_at_k};
use raki_eval::{run_eval_over, EvalQuery, Method};
use raki_retrieval::{recall_at_k, reciprocal_rank};

const K: usize = 10;
const METHODS: [(&str, Method); 4] = [
    ("kw", Method::Keyword),
    ("vec", Method::Vector),
    ("hyb", Method::Hybrid),
    ("rr", Method::Reranked),
];

#[derive(Default, Clone)]
struct Agg {
    s3: f64,
    s1: f64,
    r3: f64,
    r10: f64,
    mrr: f64,
    n: f64,
    primary_hits: f64,
    primary_n: f64,
}

fn relevant_of(q: &EvalQuery) -> HashSet<String> {
    q.relevant_ids.iter().cloned().collect()
}

fn fmt_primary(a: &Agg) -> String {
    if a.primary_n > 0.0 {
        format!(
            "{:.2} ({}/{})",
            a.primary_hits / a.primary_n,
            a.primary_n as usize,
            a.n as usize
        )
    } else {
        "n/a".to_string()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../eval-data/real");
    let data = match load_local(&dir) {
        Ok(d) => d,
        Err(e) => {
            // Helpful onboarding, clean exit — not a panic.
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // query text → its EvalQuery (for relevant_ids + primary lookup after scoring).
    let by_query: HashMap<&str, &EvalQuery> =
        data.queries.iter().map(|q| (q.query.as_str(), q)).collect();

    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let reranker = Arc::new(FastEmbedReranker::try_new()?);
    let model = embedder.model_id();
    let reranker_model = reranker.model_id();
    let run = run_eval_over(
        &data.corpus,
        &data.queries,
        embedder,
        reranker,
        K,
        raki_eval::chunk::ChunkStrategy::WholeNote,
        raki_eval::chunk::PrefixMode::Title,
        raki_eval::chunk::Rollup::MinRank,
    )
    .await?;

    // Aggregate per method (overall) and per (method, category) — category stays LOCAL only.
    let mut overall: HashMap<&str, Agg> =
        METHODS.iter().map(|(l, _)| (*l, Agg::default())).collect();
    let mut by_cat: BTreeMap<String, HashMap<&str, Agg>> = BTreeMap::new();

    println!("# real-data eval (LOCAL — not committed). k={K}\n");
    for qr in &run.per_query {
        let Some(eq) = by_query.get(qr.query.as_str()) else {
            continue;
        };
        let rel = relevant_of(eq);
        if rel.is_empty() {
            continue;
        }
        println!("[{}] {:?}", qr.category, qr.query);
        for (label, m) in METHODS {
            let ranked = &qr.method(m).ranked; // top-K ids
            let s3 = success_at_k(ranked, &rel, 3);
            let s1 = success_at_k(ranked, &rel, 1);
            let r3 = recall_at_k(ranked, &rel, 3).unwrap_or(0.0);
            let r10 = recall_at_k(ranked, &rel, K).unwrap_or(0.0);
            let mrr = reciprocal_rank(ranked, &rel).unwrap_or(0.0);
            let prim = primary_success_at_1(ranked, eq.primary.as_deref());

            for bucket in [
                overall.get_mut(label).unwrap(),
                by_cat
                    .entry(qr.category.clone())
                    .or_default()
                    .entry(label)
                    .or_default(),
            ] {
                bucket.s3 += s3;
                bucket.s1 += s1;
                bucket.r3 += r3;
                bucket.r10 += r10;
                bucket.mrr += mrr;
                bucket.n += 1.0;
                if let Some(p) = prim {
                    bucket.primary_hits += p;
                    bucket.primary_n += 1.0;
                }
            }
            println!(
                "  {label:<3} S@3 {s3:.0} S@1 {s1:.0} R@3 {r3:.2} R@10 {r10:.2} MRR {mrr:.2}{}",
                prim.map(|p| format!(" P@1 {p:.0}")).unwrap_or_default()
            );
        }
    }

    let line = |label: &str, a: &Agg| {
        let p = if a.primary_n > 0.0 {
            format!(
                " | Primary-Success@1 {:.2} (over {}/{} w/ unambiguous primary)",
                a.primary_hits / a.primary_n,
                a.primary_n as usize,
                a.n as usize
            )
        } else {
            String::new()
        };
        format!(
            "{label:<3} | Success@3 {:.2} | Success@1 {:.2} | Recall@3 {:.2} | Recall@10 {:.2} | MRR {:.2}{p}",
            a.s3 / a.n,
            a.s1 / a.n,
            a.r3 / a.n,
            a.r10 / a.n,
            a.mrr / a.n,
        )
    };

    println!("\n## Per-category (LOCAL ONLY — never committed)");
    for (cat, methods) in &by_cat {
        println!("### {cat}");
        for (label, _) in METHODS {
            println!("  {}", line(label, &methods[label]));
        }
    }

    let total_q = run
        .per_query
        .iter()
        .filter(|q| {
            by_query
                .get(q.query.as_str())
                .map(|e| !e.relevant_ids.is_empty())
                .unwrap_or(false)
        })
        .count();

    println!("\n## OVERALL ({total_q} queries)");
    for (label, _) in METHODS {
        println!("  {}", line(label, &overall[label]));
    }

    // reranked − hybrid (the bias-robust relative read; directional only).
    let d = |f: fn(&Agg) -> f64| f(&overall["rr"]) - f(&overall["hyb"]);
    println!(
        "\nreranked − hybrid (directional): ΔSuccess@3 {:+.3}  ΔMRR {:+.3}",
        d(|a| a.s3 / a.n),
        d(|a| a.mrr / a.n),
    );

    // Committed artifact: aggregate-only, content-free, with the in-band warning header.
    write_baseline(&overall, total_q, &model, &reranker_model)?;
    Ok(())
}

fn write_baseline(
    overall: &HashMap<&str, Agg>,
    total_q: usize,
    model: &str,
    reranker_model: &str,
) -> std::io::Result<()> {
    use std::fmt::Write as _;
    let dir = raki_eval::eval_dir();
    std::fs::create_dir_all(&dir)?;
    let mut s = String::new();
    s.push_str("<!-- Directional signal only. Not statistically powered; absolutes are an optimistic ceiling. See Limitations in 2026-06-06-real-data-eval-substrate-design.md. -->\n");
    s.push_str("# Real-data eval baseline (aggregate-only, content-free)\n\n");
    writeln!(s, "- Queries: {total_q}").unwrap();
    writeln!(
        s,
        "- Platform: {} / {}; embed model: `{model}`; reranker: `{reranker_model}`; k=10",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
    .unwrap();
    s.push_str("\n| method | Success@3 | Success@1 | Recall@3 | Recall@10 | MRR | Primary-Success@1 (denom) |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for (label, _) in METHODS {
        let a = &overall[label];
        let prim = fmt_primary(a);
        writeln!(
            s,
            "| {label} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {prim} |",
            a.s3 / a.n,
            a.s1 / a.n,
            a.r3 / a.n,
            a.r10 / a.n,
            a.mrr / a.n
        )
        .unwrap();
    }
    std::fs::write(dir.join("real-data-baseline.md"), s)?;
    eprintln!(
        "wrote {}/real-data-baseline.md (aggregate-only)",
        dir.display()
    );
    Ok(())
}
