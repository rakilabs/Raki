//! The FTS5-backed KeywordIndex (read path). Writes are kept in sync by the repository.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{DomainError, KeywordHit, KeywordIndex};

use crate::db::Database;

pub struct SqliteKeywordIndex {
    db: Database,
}

impl SqliteKeywordIndex {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// Turn free user text into a safe FTS5 MATCH expression: quote each whitespace-
/// separated term (doubling embedded quotes) so punctuation can't break the grammar,
/// then join with `OR` so a verbose natural-language query matches any term and bm25
/// ranks by overlap. (Space-joining is implicit AND in FTS5 — it returns nothing
/// unless one note contains every word, which crippled multi-term queries.)
/// Empty input yields an empty string, which the caller treats as "no results".
fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

#[async_trait]
impl KeywordIndex for SqliteKeywordIndex {
    async fn query(&self, query: &str, k: usize) -> Result<Vec<KeywordHit>, DomainError> {
        let match_expr = fts_query(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT note_id, bm25(notes_fts) AS score
                     FROM notes_fts
                     WHERE notes_fts MATCH ?1
                     ORDER BY score, note_id
                     LIMIT ?2",
                )?;
                let hits = stmt
                    .query_map(params![match_expr, k as i64], |row| {
                        Ok(KeywordHit {
                            source_id: row.get(0)?,
                            score: row.get::<_, f64>(1)? as f32,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(hits)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{KeywordIndex, Note, NoteId, NoteRepository};

    use crate::db::Database;
    use crate::notes::SqliteNoteRepository;

    fn note(title: &str, body: &str) -> Note {
        Note::new(title.to_string(), body.to_string(), 1000)
    }

    #[test]
    fn fts_query_quotes_each_term_and_escapes_quotes() {
        assert_eq!(fts_query("hello world"), "\"hello\" OR \"world\"");
        assert_eq!(fts_query("  spaced  "), "\"spaced\"");
        assert_eq!(fts_query(""), "");
        // a stray double-quote must not break the FTS5 grammar
        assert_eq!(fts_query("a\"b"), "\"a\"\"b\"");
    }

    #[tokio::test]
    async fn query_finds_matching_note_and_skips_others() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let index = SqliteKeywordIndex::new(db);

        let apples = note("Apples", "crisp and red");
        let oranges = note("Oranges", "citrus");
        repo.upsert(&apples).await.unwrap();
        repo.upsert(&oranges).await.unwrap();

        let hits = index.query("apples", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_id, apples.id.to_string());
    }

    #[tokio::test]
    async fn empty_query_returns_no_hits() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteKeywordIndex::new(db);
        assert!(index.query("   ", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn ties_break_by_note_id_deterministically() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let index = SqliteKeywordIndex::new(db);
        // Two notes that match "apple" identically (same single term, same length).
        let a = Note {
            id: NoteId::parse("00000000-0000-7000-8000-000000000001").unwrap(),
            title: "apple".into(),
            body: "x".into(),
            created_at: 1,
            updated_at: 1,
            deleted_at: None,
            version: 1,
        };
        let b = Note {
            id: NoteId::parse("00000000-0000-7000-8000-000000000002").unwrap(),
            title: "apple".into(),
            body: "x".into(),
            created_at: 1,
            updated_at: 1,
            deleted_at: None,
            version: 1,
        };
        repo.upsert(&b).await.unwrap();
        repo.upsert(&a).await.unwrap();
        let hits = index.query("apple", 10).await.unwrap();
        let ids: Vec<String> = hits.into_iter().map(|h| h.source_id).collect();
        assert_eq!(
            ids,
            vec![a.id.to_string(), b.id.to_string()],
            "ties ordered by note_id"
        );
    }
}
