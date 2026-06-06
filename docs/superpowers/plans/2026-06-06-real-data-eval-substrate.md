# Real-Data Eval Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the machinery to measure retrieval on the user's own Markdown notes + hand-labeled queries — local-only, binary "did I find it" metrics, with private data never committed — so retrieval quality becomes a real (if directional) signal.

**Architecture:** Extract a shared `run_eval_over(corpus, queries, …)` from `run_eval` (the deterministic keyword snapshot gate guards that the synthetic tier is unchanged). Add a `local_corpus` loader (gitignored `eval-data/real/` → Markdown→plain-text + `queries.json`) and a local `real-eval` binary that runs the four methods at k=10 and computes Success@3/@1, Recall@3/@10, MRR, Primary-Success@1, printing per-query detail locally and writing only a content-free aggregate baseline.

**Tech Stack:** Rust, `raki-eval` driver crate, `pulldown-cmark` (Markdown→text), `serde_json`, real bge model + jina reranker for local runs (reused from Slices 1–4).

**Spec:** `docs/superpowers/specs/2026-06-06-real-data-eval-substrate-design.md` (D1–D8 + Limitations). This plan implements all of it.

**Verified facts (read before starting):**
- `run_eval(embedder: Arc<dyn EmbeddingProvider>, reranker: Arc<dyn Reranker>, k: usize) -> Result<EvalRun, DomainError>` in `crates/raki-eval/src/lib.rs:~233`. It calls `load_corpus()` / `load_queries()` then builds an in-memory index. `EvalRun { report, per_query: Vec<QueryResult> }`. `QueryResult { query, category, set, keyword, vector, hybrid, reranked: MethodResult }`; `MethodResult { ranked: Vec<String> /* top-k fixture ids */, scores }`. `CorpusNote { id, title, body }`, `EvalQuery { query, category: String, set, relevant_ids: Vec<String>, grades }`.
- Metric primitives in `raki-retrieval`: `recall_at_k(&[String], &HashSet<String>, usize) -> Option<f64>`, `reciprocal_rank(&[String], &HashSet<String>) -> Option<f64>`. (No `success_at_k`/`primary` — we add tiny helpers.)
- `crates/raki-eval/Cargo.toml` has one `[[bin]] name = "eval-report" path = "src/main.rs"` and deps: raki-{domain,storage,ai,retrieval}, serde, serde_json, tokio. No `pulldown-cmark`.
- The deterministic gate `keyword_snapshot_is_deterministic` (in `tests/eval_gate.rs`, NOT `#[ignore]`) pins exact per-query keyword rankings — it is the refactor guard for Task 1.

---

## File Structure

```
raki-eval/src/lib.rs                 MODIFY  extract run_eval_over; EvalQuery gains optional `primary` + default `category`
raki-eval/src/markdown.rs            CREATE  pure Markdown→plain-text (strip frontmatter, drop HTML, code text only)
raki-eval/src/local_corpus.rs        CREATE  load notes dir + queries.json → (corpus, queries); resolution invariant; missing-dir helpful error
raki-eval/src/realmetrics.rs         CREATE  success_at_k, primary_success_at_1 (+ thin re-use of recall/mrr)
raki-eval/src/bin/real-eval.rs       CREATE  local binary: load → run_eval_over(k=10) → metrics → local detail + aggregate baseline
raki-eval/Cargo.toml                 MODIFY  + pulldown-cmark dep; + [[bin]] real-eval
raki-eval/tests/fixtures/local/      CREATE  COMMITTED synthetic fixture (notes/*.md + queries.json) for loader tests — NOT real data
.gitignore                           MODIFY  + eval-data/
docs/eval/real-data-baseline.md      CREATE  committed aggregate-only template (warning header; "not yet run")
docs/eval/real-data-protocol.md      CREATE  D6 discipline / D7 privacy / D8 cadence
```

The synthetic 30-note tier, `snapshot.json`, the gate, and `search_notes` are **untouched**.

---

## Task 1: Extract `run_eval_over` (refactor, guarded by the keyword snapshot gate)

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Confirm the guard is green before refactoring**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS. This is the characterization guard — it must still pass after the extraction.

- [ ] **Step 2: Extract the body into `run_eval_over`, make `run_eval` delegate**

In `lib.rs`, replace the `run_eval` function header + its first two lines (the loads) so the signature becomes a thin wrapper and a new `run_eval_over` takes the data. Concretely, change:

```rust
pub async fn run_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    let corpus = load_corpus();
    let queries = load_queries();

    let db = Database::open_in_memory()?;
    // ... rest of body ...
}
```

to:

```rust
/// Run the eval over a CALLER-SUPPLIED corpus + queries (the synthetic fixtures, or a local
/// real-notes set). All retrieval/scoring logic lives here; `run_eval` is the fixture wrapper.
pub async fn run_eval_over(
    corpus: Vec<CorpusNote>,
    queries: Vec<EvalQuery>,
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    let db = Database::open_in_memory()?;
    // ... ENTIRE existing body that followed the two load calls, unchanged ...
}

/// Eval over the committed synthetic fixtures (the smoke/regression tier).
pub async fn run_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    run_eval_over(load_corpus(), load_queries(), embedder, reranker, k).await
}
```

Move everything from the original `let db = Database::open_in_memory()?;` through the final `Ok(EvalRun { … })` verbatim into `run_eval_over`. Delete the two `load_*` lines from the old location (they now live only in `run_eval`).

- [ ] **Step 3: Build + run the full eval test set (behavior preserved)**

Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS — including `keyword_snapshot_is_deterministic` (proves the extraction changed no behavior on the synthetic tier) and `harness_scores_every_category_with_fake_embedder`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Extract run_eval_over from run_eval (keyword-snapshot-guarded refactor)"
```

---

## Task 2: `EvalQuery` gains optional `primary` + default `category`

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `lib.rs` `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn eval_query_parses_optional_primary_and_default_category() {
        // category omitted → defaults; primary present.
        let q: EvalQuery = serde_json::from_str(
            r#"{ "query": "q", "relevant_ids": ["a","b"], "primary": "a" }"#,
        )
        .unwrap();
        assert_eq!(q.category, "real");
        assert_eq!(q.primary.as_deref(), Some("a"));
        // primary omitted → None.
        let q2: EvalQuery =
            serde_json::from_str(r#"{ "query": "q2", "category": "exact", "relevant_ids": ["c"] }"#)
                .unwrap();
        assert_eq!(q2.category, "exact");
        assert_eq!(q2.primary, None);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-eval --lib eval_query_parses_optional_primary`
Expected: FAIL — `category` is currently required (no default) and `primary` doesn't exist.

- [ ] **Step 3: Add the fields**

In `lib.rs`, update `EvalQuery` (around line 16-28). Change `category` to default and add `primary`:

```rust
#[derive(Debug, Deserialize)]
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
```

(The existing synthetic fixtures all specify `category`, so the default is inert for them; `primary` is unused there.)

- [ ] **Step 4: Run the test + the full lib tests**

Run: `cd src-tauri && cargo test -p raki-eval --lib`
Expected: PASS (new test green; existing loader tests still parse the synthetic fixtures unchanged).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "EvalQuery: optional primary + default category (real-data tier)"
```

---

## Task 3: Markdown → plain-text extraction (pure, fidelity-tested)

**Files:**
- Modify: `src-tauri/crates/raki-eval/Cargo.toml`
- Create: `src-tauri/crates/raki-eval/src/markdown.rs`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs` (add `mod markdown;`)

- [ ] **Step 1: Add the dependency**

In `crates/raki-eval/Cargo.toml`, under `[dependencies]`, add:

```toml
pulldown-cmark = { version = "0.12", default-features = false }
```

- [ ] **Step 2: Create the module with the failing test**

Create `src-tauri/crates/raki-eval/src/markdown.rs`:

```rust
//! Markdown → plain text for the real-data eval. Strips YAML frontmatter, drops HTML, and
//! emits code-block *contents* without fences or language tags. A deliberate approximation of
//! the eventual ProseMirror block-aware pipeline (see the real-data spec, Limitations).

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// Strip a leading `---\n … \n---` YAML frontmatter block, if present.
fn strip_frontmatter(src: &str) -> &str {
    let Some(rest) = src.strip_prefix("---\n") else {
        return src;
    };
    // End delimiter: a line that is exactly `---`.
    if let Some(end) = rest.find("\n---\n") {
        &rest[end + 5..]
    } else if let Some(end) = rest.find("\n---") {
        &rest[end + 4..]
    } else {
        src
    }
}

/// Extract readable text: Text + inline Code + code-block text; HTML events are dropped;
/// block boundaries become single spaces. Collapses runs of whitespace.
pub fn to_plain_text(md: &str) -> String {
    let body = strip_frontmatter(md);
    let mut out = String::with_capacity(body.len());
    for event in Parser::new(body) {
        match event {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Item)
            | Event::End(TagEnd::CodeBlock) => out.push(' '),
            Event::Start(Tag::CodeBlock(_)) => out.push(' '),
            // Event::Html / InlineHtml deliberately dropped (no tag leakage).
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first level-1 heading's text, or `None` if the doc has no H1.
pub fn first_h1(md: &str) -> Option<String> {
    let body = strip_frontmatter(md);
    let mut in_h1 = false;
    let mut title = String::new();
    for event in Parser::new(body) {
        match event {
            Event::Start(Tag::Heading { level: pulldown_cmark::HeadingLevel::H1, .. }) => {
                in_h1 = true;
            }
            Event::Text(t) if in_h1 => title.push_str(&t),
            Event::End(TagEnd::Heading(pulldown_cmark::HeadingLevel::H1)) if in_h1 => {
                return Some(title.trim().to_string());
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRICKY: &str = "---\ntitle: Frontmatter Note\ntags: [a, b]\n---\n# Real Title\n\nText with a [[WikiLink]] and <b>html</b> inline.\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";

    #[test]
    fn extracts_text_without_frontmatter_html_or_fence_noise() {
        let text = to_plain_text(TRICKY);
        // Code block contents survive, fences + language tag do not.
        assert!(text.contains("fn main() { println!(\"hi\"); }"));
        assert!(!text.contains("```"));
        assert!(!text.contains("rust fn main"), "language id must not prefix code");
        // HTML tags dropped (no leakage); inner text may remain.
        assert!(!text.contains("<b>"));
        // Frontmatter keys are gone.
        assert!(!text.contains("tags:"));
        assert!(!text.contains("title: Frontmatter"));
        // Wikilink target text is preserved as content.
        assert!(text.contains("WikiLink"));
    }

    #[test]
    fn first_h1_is_the_title() {
        assert_eq!(first_h1(TRICKY).as_deref(), Some("Real Title"));
        assert_eq!(first_h1("no heading here").as_deref(), None);
    }
}
```

- [ ] **Step 3: Wire the module**

In `lib.rs`, add `mod markdown;` near the top (with the other module/`use` declarations) — it is used by `local_corpus` (Task 4), so no `pub` re-export needed yet, but mark it `pub mod markdown;` to keep it test-visible and reusable.

- [ ] **Step 4: Run the tests**

Run: `cd src-tauri && cargo test -p raki-eval --lib markdown`
Expected: PASS (both tests). If `pulldown-cmark` 0.12's `Tag`/`TagEnd` enum shapes differ in a patch, adjust the match arms to the installed version (the event kinds — Text, Code, Html, Heading, CodeBlock, Paragraph, Item — are stable).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/Cargo.toml src-tauri/crates/raki-eval/src/markdown.rs src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Add Markdown->plain-text extraction for the real-data eval"
```

---

## Task 4: `local_corpus` loader (notes dir + queries.json, invariant, helpful missing-dir error)

**Files:**
- Create: `src-tauri/crates/raki-eval/src/local_corpus.rs`
- Create (committed test fixture): `src-tauri/crates/raki-eval/tests/fixtures/local/notes/alpha.md`, `.../beta.md`, `.../queries.json`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs` (add `pub mod local_corpus;`)

- [ ] **Step 1: Create the committed synthetic fixture (NOT real data)**

Create `src-tauri/crates/raki-eval/tests/fixtures/local/notes/alpha.md`:

```markdown
# Espresso dialing

If the shot tastes sour, grind finer and aim for a 1:2 ratio over about 28 seconds.
```

Create `src-tauri/crates/raki-eval/tests/fixtures/local/notes/beta.md`:

```markdown
# Sourdough rise

Dense, flat loaves usually mean under-proofing or a weak starter. Ferment longer.
```

Create `src-tauri/crates/raki-eval/tests/fixtures/local/queries.json`:

```json
[
  { "query": "why is my espresso sour", "relevant_ids": ["alpha"], "primary": "alpha", "category": "vague" },
  { "query": "bread did not rise", "relevant_ids": ["beta"], "category": "exact" }
]
```

- [ ] **Step 2: Create the loader with failing tests**

Create `src-tauri/crates/raki-eval/src/local_corpus.rs`:

```rust
//! Loads the LOCAL, gitignored real-data eval set: a directory of Markdown notes plus a
//! `queries.json`. Private data lives under `eval-data/real/` and is never committed; this
//! loader is also pointed at a committed synthetic fixture dir by its tests.

use std::collections::HashSet;
use std::path::Path;

use crate::markdown::{first_h1, to_plain_text};
use crate::{CorpusNote, EvalQuery};

/// What `load_local` returns: the parsed corpus + queries, ready for `run_eval_over`.
pub struct LocalData {
    pub corpus: Vec<CorpusNote>,
    pub queries: Vec<EvalQuery>,
}

#[derive(Debug)]
pub enum LoadError {
    Missing(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Unresolved(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Missing(p) => write!(
                f,
                "real-data eval not set up: {p} not found.\n\
                 To set up:\n\
                 1. Export your notes to eval-data/real/notes/*.md\n\
                 2. Author queries in eval-data/real/queries.json\n\
                 3. See docs/eval/real-data-protocol.md"
            ),
            LoadError::Io(e) => write!(f, "io error: {e}"),
            LoadError::Json(e) => write!(f, "queries.json parse error: {e}"),
            LoadError::Unresolved(m) => write!(f, "label resolution error: {m}"),
        }
    }
}
impl std::error::Error for LoadError {}

/// Slug = file stem (e.g. `my-note.md` → `my-note`), the stable note id used in labels.
fn slug(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string()
}

/// Load notes from `dir/notes/*.md` and queries from `dir/queries.json`. Returns a helpful
/// `Missing` error if `dir` or `dir/notes` is absent (turns a crash into onboarding).
pub fn load_local(dir: &Path) -> Result<LocalData, LoadError> {
    let notes_dir = dir.join("notes");
    if !notes_dir.is_dir() {
        return Err(LoadError::Missing(notes_dir.display().to_string()));
    }
    let mut corpus = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&notes_dir)
        .map_err(LoadError::Io)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    entries.sort(); // deterministic order
    for path in entries {
        let raw = std::fs::read_to_string(&path).map_err(LoadError::Io)?;
        let id = slug(&path);
        let title = first_h1(&raw).unwrap_or_else(|| id.clone());
        corpus.push(CorpusNote { id, title, body: to_plain_text(&raw) });
    }

    let queries_path = dir.join("queries.json");
    if !queries_path.is_file() {
        return Err(LoadError::Missing(queries_path.display().to_string()));
    }
    let qtext = std::fs::read_to_string(&queries_path).map_err(LoadError::Io)?;
    let queries: Vec<EvalQuery> = serde_json::from_str(&qtext).map_err(LoadError::Json)?;

    validate(&corpus, &queries)?;
    Ok(LocalData { corpus, queries })
}

/// Every `relevant_id` and `primary` must resolve to a real note slug.
fn validate(corpus: &[CorpusNote], queries: &[EvalQuery]) -> Result<(), LoadError> {
    let ids: HashSet<&str> = corpus.iter().map(|n| n.id.as_str()).collect();
    for q in queries {
        for r in &q.relevant_ids {
            if !ids.contains(r.as_str()) {
                return Err(LoadError::Unresolved(format!(
                    "query {:?}: relevant_id {r:?} matches no note",
                    q.query
                )));
            }
        }
        if let Some(p) = &q.primary {
            if !ids.contains(p.as_str()) {
                return Err(LoadError::Unresolved(format!(
                    "query {:?}: primary {p:?} matches no note",
                    q.query
                )));
            }
            if !q.relevant_ids.iter().any(|r| r == p) {
                return Err(LoadError::Unresolved(format!(
                    "query {:?}: primary {p:?} must also be in relevant_ids",
                    q.query
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/local")
    }

    #[test]
    fn loads_notes_and_queries_with_resolved_labels() {
        let data = load_local(&fixture_dir()).unwrap();
        assert_eq!(data.corpus.len(), 2);
        let alpha = data.corpus.iter().find(|n| n.id == "alpha").unwrap();
        assert_eq!(alpha.title, "Espresso dialing");
        assert!(alpha.body.contains("grind finer"));
        assert_eq!(data.queries.len(), 2);
        let q = data.queries.iter().find(|q| q.primary.is_some()).unwrap();
        assert_eq!(q.primary.as_deref(), Some("alpha"));
        assert_eq!(q.category, "vague");
    }

    #[test]
    fn missing_dir_is_a_helpful_error_not_a_panic() {
        let err = load_local(std::path::Path::new("/nonexistent/eval-data/real")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not set up"));
        assert!(msg.contains("real-data-protocol.md"));
    }
}
```

- [ ] **Step 3: Wire the module**

In `lib.rs`, add `pub mod local_corpus;` with the other module declarations. (`CorpusNote` and `EvalQuery` are already `pub` in `lib.rs`, so the loader's `use crate::{CorpusNote, EvalQuery}` resolves.)

- [ ] **Step 4: Run the tests**

Run: `cd src-tauri && cargo test -p raki-eval --lib local_corpus`
Expected: PASS (loads fixture, resolves labels, helpful missing-dir error).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/src/local_corpus.rs src-tauri/crates/raki-eval/src/lib.rs src-tauri/crates/raki-eval/tests/fixtures/local
git commit -m "Add local_corpus loader (markdown dir + queries.json, resolution invariant)"
```

---

## Task 5: Binary metric helpers (Success@k, Primary-Success@1)

**Files:**
- Create: `src-tauri/crates/raki-eval/src/realmetrics.rs`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs` (add `pub mod realmetrics;`)

- [ ] **Step 1: Create the module with failing tests**

Create `src-tauri/crates/raki-eval/src/realmetrics.rs`:

```rust
//! Binary "did I find it" metrics for the real-data tier. Recall/MRR reuse the
//! `raki-retrieval` primitives; these two are the additions (Success@k, Primary-Success@1).

use std::collections::HashSet;

/// 1.0 if any relevant id appears in the top-`k`, else 0.0.
pub fn success_at_k(ranked: &[String], relevant: &HashSet<String>, k: usize) -> f64 {
    if ranked.iter().take(k).any(|id| relevant.contains(id)) {
        1.0
    } else {
        0.0
    }
}

/// 1.0 if the (unambiguous) `primary` note is ranked #1; `None` when the query marks no
/// primary (so it is excluded from the Primary-Success@1 aggregate + its denominator).
pub fn primary_success_at_1(ranked: &[String], primary: Option<&str>) -> Option<f64> {
    let p = primary?;
    Some(if ranked.first().map(|s| s.as_str()) == Some(p) {
        1.0
    } else {
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    fn rel(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn success_at_k_single_and_multi_relevant() {
        let ranked = ids(&["x", "a", "y"]);
        assert_eq!(success_at_k(&ranked, &rel(&["a"]), 3), 1.0);
        assert_eq!(success_at_k(&ranked, &rel(&["a"]), 1), 0.0); // a is at rank 2
        // multi-relevant: any one in top-k counts.
        assert_eq!(success_at_k(&ranked, &rel(&["a", "b"]), 3), 1.0);
        assert_eq!(success_at_k(&ranked, &rel(&["z"]), 3), 0.0);
    }

    #[test]
    fn primary_success_is_top1_only_and_opt_out() {
        let ranked = ids(&["a", "b"]);
        assert_eq!(primary_success_at_1(&ranked, Some("a")), Some(1.0));
        assert_eq!(primary_success_at_1(&ranked, Some("b")), Some(0.0));
        assert_eq!(primary_success_at_1(&ranked, None), None); // excluded from the metric
    }
}
```

- [ ] **Step 2: Wire + run**

Add `pub mod realmetrics;` to `lib.rs`. Run: `cd src-tauri && cargo test -p raki-eval --lib realmetrics`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-eval/src/realmetrics.rs src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Add Success@k + Primary-Success@1 metric helpers"
```

---

## Task 6: The `real-eval` local binary

**Files:**
- Create: `src-tauri/crates/raki-eval/src/bin/real-eval.rs`
- Modify: `src-tauri/crates/raki-eval/Cargo.toml` (add `[[bin]]`)

- [ ] **Step 1: Register the binary**

In `crates/raki-eval/Cargo.toml`, after the existing `[[bin]]` block, add:

```toml
[[bin]]
name = "real-eval"
path = "src/bin/real-eval.rs"
```

- [ ] **Step 2: Write the binary**

Create `src-tauri/crates/raki-eval/src/bin/real-eval.rs`:

```rust
//! `real-eval`: LOCAL-ONLY measurement on real Markdown notes + labeled queries under
//! `eval-data/real/` (gitignored). Prints per-method / per-query / per-category detail to the
//! terminal (never written to git) and writes ONLY a content-free aggregate baseline.
//! Directional signal — not statistically powered. See the real-data spec, Limitations.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_domain::EmbeddingProvider;
use raki_eval::local_corpus::load_local;
use raki_eval::realmetrics::{primary_success_at_1, success_at_k};
use raki_eval::{run_eval_over, EvalQuery, Method, QueryResult};
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
    let run = run_eval_over(data.corpus, data.queries.clone(), embedder, reranker, K).await?;

    // Aggregate per method (overall) and per (method, category) — category stays LOCAL only.
    let mut overall: HashMap<&str, Agg> = METHODS.iter().map(|(l, _)| (*l, Agg::default())).collect();
    let mut by_cat: BTreeMap<String, HashMap<&str, Agg>> = BTreeMap::new();

    println!("# real-data eval (LOCAL — not committed). k={K}\n");
    for qr in &run.per_query {
        let Some(eq) = by_query.get(qr.query.as_str()) else { continue };
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

    let total_q = run.per_query.iter().filter(|q| {
        by_query
            .get(q.query.as_str())
            .map(|e| !e.relevant_ids.is_empty())
            .unwrap_or(false)
    }).count();

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
    write_baseline(&overall, total_q, &model)?;
    Ok(())
}

fn write_baseline(
    overall: &HashMap<&str, Agg>,
    total_q: usize,
    model: &str,
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
        "- Platform: {} / {}; embed model: `{model}`; reranker: `jina-reranker-v1-turbo-en`; k=10",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
    .unwrap();
    s.push_str("\n| method | Success@3 | Success@1 | Recall@3 | Recall@10 | MRR | Primary-Success@1 (denom) |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for (label, _) in METHODS {
        let a = &overall[label];
        let prim = if a.primary_n > 0.0 {
            format!("{:.2} ({}/{})", a.primary_hits / a.primary_n, a.primary_n as usize, a.n as usize)
        } else {
            "n/a".to_string()
        };
        writeln!(
            s,
            "| {label} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {prim} |",
            a.s3 / a.n, a.s1 / a.n, a.r3 / a.n, a.r10 / a.n, a.mrr / a.n
        )
        .unwrap();
    }
    std::fs::write(dir.join("real-data-baseline.md"), s)?;
    eprintln!("wrote {}/real-data-baseline.md (aggregate-only)", dir.display());
    Ok(())
}
```

(Confirm `Method` and `eval_dir` are `pub` in `raki_eval` — they are, from Slices 3a-ii/4. `QueryResult` import is used via `run.per_query`.)

- [ ] **Step 3: Verify it compiles**

Run: `cd src-tauri && cargo build -p raki-eval --bin real-eval`
Expected: builds clean. (Running it needs a populated `eval-data/real/`; with none present it prints the onboarding message and exits 2 — verify in Task 8.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-eval/Cargo.toml src-tauri/crates/raki-eval/src/bin/real-eval.rs
git commit -m "Add real-eval local binary (find-it metrics, aggregate-only baseline)"
```

---

## Task 7: Privacy plumbing + docs (gitignore, protocol, baseline template)

**Files:**
- Modify: `.gitignore`
- Create: `docs/eval/real-data-protocol.md`
- Create: `docs/eval/real-data-baseline.md`

- [ ] **Step 1: Gitignore the private data dir**

Append to `.gitignore` (repo root):

```gitignore
# Real-data eval: private personal notes + labeled queries. NEVER commit.
eval-data/
```

- [ ] **Step 2: Write the protocol doc**

Create `docs/eval/real-data-protocol.md`:

```markdown
# Real-data eval protocol (local, private)

Machinery: `cargo run -p raki-eval --bin real-eval` (real model; reads `eval-data/real/`).
Setup: put `.md` notes in `eval-data/real/notes/`, write `eval-data/real/queries.json`
(`[{ "query", "relevant_ids": ["note-slug"], "primary"?: "slug", "category"?: "..." }]`).

## Labeling discipline (D6) — highest-leverage first
1. **Query like a vague future self** — half-remembered, approximate terms, NEVER the note's
   exact words. This is the primary anti-bias action; absolutes remain an optimistic ceiling.
2. **Author + label from memory, before running retrieval.**
3. **Short wait** (a few hours) before running — tertiary.
4. **Phase-2 pooling to ~top-20**: after a run, add any *additional* genuinely-correct note you
   missed; never label toward what ranked highly. Incomplete pooling biases metrics *down* (safe).
5. Mark `primary` ONLY when there is an unambiguous single best answer.

## Privacy (D7)
- `eval-data/` is gitignored; notes + queries + per-query/per-category detail are LOCAL ONLY.
- Only `docs/eval/real-data-baseline.md` is committed — aggregate metrics only, no content.
- Risk: `git add -f eval-data/` would leak private data — do not force-add it.

## Cadence (D8)
Re-run **monthly for the first quarter, then quarterly**, updating the baseline. A lapsed
cadence is the main way this eval rots.

## What the numbers are NOT
Directional, not statistically powered (~20–40 queries); optimistic ceiling (authorship bias);
whole-note plain text, not block-aware. Do not decide the reranker's fate (D-DELETE) on this set.
```

- [ ] **Step 3: Write the committed baseline template (pre-run)**

Create `docs/eval/real-data-baseline.md`:

```markdown
<!-- Directional signal only. Not statistically powered; absolutes are an optimistic ceiling. See Limitations in 2026-06-06-real-data-eval-substrate-design.md. -->
# Real-data eval baseline (aggregate-only, content-free)

Not yet run. Populate `eval-data/real/` (see `real-data-protocol.md`) and run
`cargo run -p raki-eval --bin real-eval`; it overwrites this file with aggregate-only metrics.
```

- [ ] **Step 4: Confirm the gitignore actually hides the data dir**

Run: `mkdir -p eval-data/real/notes && touch eval-data/real/notes/secret.md && git status --porcelain eval-data; rm -rf eval-data`
Expected: **no output** from `git status` (the dir is ignored). If `eval-data/` shows up, the gitignore entry is wrong — fix before committing.

- [ ] **Step 5: Commit**

```bash
git add .gitignore docs/eval/real-data-protocol.md docs/eval/real-data-baseline.md
git commit -m "Real-data eval: gitignore private data, protocol + baseline-template docs"
```

---

## Task 8: Verification + Definition of Done

- [ ] **Step 1: Full deterministic sweep (mirrors required CI)**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo fmt --check && cargo clippy --workspace --exclude raki --all-targets -- -D warnings`
Expected: all pass, clean. Confirms the new modules/tests are green and the refactor didn't break the workspace.

- [ ] **Step 2: The keyword snapshot gate still passes (refactor safety)**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS — `run_eval_over` extraction preserved synthetic-tier behavior.

- [ ] **Step 3: The binary's onboarding path works (no private data needed)**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin real-eval`
Expected: prints the "real-data eval not set up … see docs/eval/real-data-protocol.md" message and exits non-zero (no panic, no crash) — because `eval-data/real/` does not exist.

- [ ] **Step 4: End-to-end smoke on the committed fixture (optional, real model)**

Temporarily point the binary at the test fixture to prove the full path runs end-to-end:
`cd src-tauri && cp -r crates/raki-eval/tests/fixtures/local eval-data/real && cargo run -q -p raki-eval --bin real-eval; rm -rf eval-data`
Expected: prints per-query + per-category + OVERALL tables and writes `docs/eval/real-data-baseline.md`. **Then restore the committed template:** `git checkout docs/eval/real-data-baseline.md` (the smoke run overwrote it with fixture numbers — do not commit those).

- [ ] **Step 5: Confirm no private data path is committed**

Run (repo root): `git status --porcelain && git ls-files eval-data` 
Expected: `git ls-files eval-data` prints nothing (no data tracked); working tree clean (the smoke run's `eval-data/` was removed and the baseline template restored).

- [ ] **Step 6: DoD against the spec**

D1 (`run_eval_over`, keyword-guarded) ✓ Task 1 · D2 (gitignored layout, slug ids, invariant) ✓ Tasks 4,7 · D3 (frontmatter strip + `pulldown-cmark`, fidelity-tested) ✓ Task 3 · D4 (binary Success@3 headline + Recall/MRR + Primary w/ denominator) ✓ Tasks 5,6 · D5 (per-method + `reranked−hybrid` directional) ✓ Task 6 · D6 (protocol, vague-future-self first) ✓ Task 7 · D7 (aggregate-only, in-band header, gitignore) ✓ Tasks 6,7 · D8 (cadence doc) ✓ Task 7 · Limitations framing ✓ (header + protocol doc). Synthetic tier / gate / `search_notes` untouched ✓ (no edits to those files). Frontend untouched.

- [ ] **Step 7: Frontend sanity**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (no frontend files changed).

---

## Self-Review

**Spec coverage:** D1 → Task 1 (extraction + guard). D2 → Tasks 4 (loader, slug ids, invariant) + 7 (gitignore). D3 → Task 3 (`pulldown-cmark`, frontmatter strip, fidelity test). D4 → Tasks 5 (Success/Primary helpers) + 6 (computed + reported, Success@3 headline, Primary with denominator). D5 → Task 6 (per-method table + `reranked−hybrid`). D6 → Task 7 (protocol, vague-future-self first, top-20 pooling). D7 → Tasks 6 (aggregate-only baseline + in-band header) + 7 (gitignore + risk note). D8 → Task 7 (cadence). Limitations → the committed header comment + protocol "What the numbers are NOT". Helpful missing-dir error → Tasks 4 + 8 Step 3.

**Placeholder scan:** none. Every code step is complete; the only runtime "unknowns" are the user's own notes/queries (by design — the slice ships machinery, the user supplies data).

**Type/consistency:** `run_eval_over(corpus: Vec<CorpusNote>, queries: Vec<EvalQuery>, embedder, reranker, k)` defined in Task 1, called identically in Task 6. `EvalQuery.primary: Option<String>` / default `category` (Task 2) used by the loader validation (Task 4) and the binary's `eq.primary.as_deref()` (Task 6). `success_at_k(&[String], &HashSet<String>, usize)` / `primary_success_at_1(&[String], Option<&str>)` (Task 5) called with those exact types in Task 6. `Method` variants (`Keyword/Vector/Hybrid/Reranked`) and `qr.method(m).ranked` match Slice 4's definitions. The binary reuses `recall_at_k` / `reciprocal_rank` (raki-retrieval) and `eval_dir` (raki-eval) — both confirmed `pub`. Note slugs (file stems) are the ids that `relevant_ids` reference — consistent between loader (Task 4) and fixture (Task 4 Step 1).

**Known approximation (by design, per spec):** k=10 retrieval depth gives the top-10 needed for Recall@10/MRR while Success@3/@1/Recall@3 read the head of the same list — one run, all metrics.

---

## Execution Handoff

(Presented to the user after saving.)
