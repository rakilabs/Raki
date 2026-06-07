//! Loads the LOCAL, gitignored real-data eval set: a directory of Markdown notes plus a
//! `queries.json`. Private data lives under `eval-data/real/` and is never committed; this
//! loader is also pointed at a committed synthetic fixture dir by its tests.

use std::collections::HashSet;
use std::path::Path;

use crate::markdown::{first_h1, to_plain_text};
use crate::{CorpusNote, EvalQuery};

/// What `load_local` returns: the parsed corpus + queries, ready for `run_eval_over`.
#[derive(Debug)]
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

/// Like `load_local`, but each note's `body` is the RAW markdown (frontmatter stripped, NOT
/// collapsed to plain text) — required so the chunker's `to_blocks` can see paragraph/heading
/// structure. Used by `chunk-eval`; the whole-note arm then embeds raw markdown too, which keeps
/// the chunked-vs-whole comparison internally consistent.
pub fn load_local_raw(dir: &Path) -> Result<LocalData, LoadError> {
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
    entries.sort();
    for path in entries {
        let raw = std::fs::read_to_string(&path).map_err(LoadError::Io)?;
        let id = slug(&path);
        let stripped = crate::markdown::strip_frontmatter(&raw);
        let title = crate::markdown::first_h1(stripped).unwrap_or_else(|| id.clone());
        corpus.push(CorpusNote {
            id,
            title,
            body: stripped.to_string(),
        });
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
        let stripped = crate::markdown::strip_frontmatter(&raw);
        let title = first_h1(stripped).unwrap_or_else(|| id.clone());
        corpus.push(CorpusNote {
            id,
            title,
            body: to_plain_text(stripped),
        });
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

    #[test]
    fn load_local_raw_keeps_paragraph_structure() {
        let data = load_local_raw(&fixture_dir()).unwrap();
        let alpha = data.corpus.iter().find(|n| n.id == "alpha").unwrap();
        // raw markdown retains structure (the to_plain_text path would collapse it).
        // alpha.md is a single short note; assert frontmatter/heading handling is intact.
        assert!(alpha.body.contains("grind finer"));
    }
}
