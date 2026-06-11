//! The sqlite-vec-backed VectorIndex. Vectors are stored as compact little-endian
//! f32 blobs in the `chunk_vectors` vec0 table (declared `float[384]`).

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{DomainError, Embedding, VectorHit, VectorIndex};

use crate::db::Database;

pub struct SqliteVectorIndex {
    db: Database,
}

impl SqliteVectorIndex {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// vec0 stores float32 vectors as a raw little-endian f32 byte blob. Building it by
/// hand keeps us off the (alpha-stage) zerocopy dependency.
fn embedding_to_blob(e: &Embedding) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(e.0.len() * 4);
    for x in &e.0 {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    bytes
}

#[async_trait]
impl VectorIndex for SqliteVectorIndex {
    async fn upsert(&self, source_id: &str, embedding: &Embedding) -> Result<(), DomainError> {
        let id = source_id.to_string();
        let blob = embedding_to_blob(embedding);
        self.db
            .call(move |c| {
                // vec0 has no UPSERT; delete+insert overwrites by primary key.
                let tx = c.unchecked_transaction()?;
                tx.execute("DELETE FROM chunk_vectors WHERE chunk_id = ?1", params![id])?;
                tx.execute(
                    "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
                    params![id, blob],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn query(&self, embedding: &Embedding, k: usize) -> Result<Vec<VectorHit>, DomainError> {
        let blob = embedding_to_blob(embedding);
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT chunk_id, distance
                     FROM chunk_vectors
                     WHERE embedding MATCH ?1 AND k = ?2
                     ORDER BY distance",
                )?;
                let hits = stmt
                    .query_map(params![blob, k as i64], |row| {
                        Ok(VectorHit {
                            source_id: row.get(0)?,
                            distance: row.get::<_, f64>(1)? as f32,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(hits)
            })
            .await
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<(), DomainError> {
        let p = prefix.to_string();
        self.db
            .call(move |c| {
                c.execute(
                    "DELETE FROM chunk_vectors WHERE chunk_id LIKE ?1 || '%'",
                    params![p],
                )?;
                Ok(())
            })
            .await
    }

    async fn upsert_batch(&self, items: &[(String, Embedding)]) -> Result<(), DomainError> {
        let items: Vec<(String, Vec<u8>)> = items
            .iter()
            .map(|(id, emb)| (id.clone(), embedding_to_blob(emb)))
            .collect();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                for (id, blob) in &items {
                    tx.execute("DELETE FROM chunk_vectors WHERE chunk_id = ?1", params![id])?;
                    tx.execute(
                        "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
                        params![id, blob],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{Embedding, VectorIndex};

    use crate::db::Database;

    /// A 384-dim basis vector: all zeros except a 1.0 at position `i`.
    fn basis(i: usize) -> Embedding {
        let mut v = vec![0.0_f32; 384];
        v[i] = 1.0;
        Embedding(v)
    }

    #[tokio::test]
    async fn upsert_then_query_returns_nearest_first() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db);
        index.upsert("a#0", &basis(0)).await.unwrap();
        index.upsert("b#0", &basis(1)).await.unwrap();
        index.upsert("c#0", &basis(2)).await.unwrap();

        let hits = index.query(&basis(1), 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].source_id, "b#0", "exact match ranks first");
    }

    #[tokio::test]
    async fn upsert_is_idempotent_overwrite() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db.clone());
        index.upsert("a#0", &basis(0)).await.unwrap();
        index.upsert("a#0", &basis(5)).await.unwrap(); // overwrite, not duplicate

        let n: i64 = db
            .call(|c| c.query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 1, "re-upserting the same id overwrites");
    }

    #[tokio::test]
    async fn query_limits_to_k() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db);
        for i in 0..5 {
            index.upsert(&format!("n{i}#0"), &basis(i)).await.unwrap();
        }
        let hits = index.query(&basis(0), 2).await.unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn delete_by_prefix_removes_matching_chunks() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db.clone());
        index.upsert("note-a#0", &basis(0)).await.unwrap();
        index.upsert("note-a#1", &basis(1)).await.unwrap();
        index.upsert("note-b#0", &basis(2)).await.unwrap();

        index.delete_by_prefix("note-a#").await.unwrap();

        let hits = index.query(&basis(0), 10).await.unwrap();
        let ids: Vec<_> = hits.iter().map(|h| h.source_id.as_str()).collect();
        assert!(!ids.contains(&"note-a#0"));
        assert!(!ids.contains(&"note-a#1"));
        assert!(ids.contains(&"note-b#0"));
    }

    #[tokio::test]
    async fn upsert_batch_writes_all_chunks() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db.clone());
        let items = vec![
            ("chunk#0".to_string(), basis(0)),
            ("chunk#1".to_string(), basis(1)),
            ("chunk#2".to_string(), basis(2)),
        ];
        index.upsert_batch(&items).await.unwrap();

        let hits = index.query(&basis(1), 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].source_id, "chunk#1");
    }
}
