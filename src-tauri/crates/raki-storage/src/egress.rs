//! SQLite adapters for the egress audit log and the consent/mode settings.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{
    DomainError, EgressDecision, EgressLog, EgressLogId, EgressRecord, EgressSettings, SourceId,
};

use crate::db::Database;

pub struct SqliteEgressLog {
    db: Database,
}
impl SqliteEgressLog {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

pub struct SqliteEgressSettings {
    db: Database,
}
impl SqliteEgressSettings {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl EgressLog for SqliteEgressLog {
    async fn record(&self, rec: &EgressRecord) -> Result<(), DomainError> {
        let id = rec.id.to_string();
        let created_at = rec.completed_at;
        let provider = rec.decision.provider.clone();
        let model = rec.decision.model.clone();
        let token_count = rec.decision.total_tokens as i64;
        let source_ids = serde_json::to_string(&rec.decision.source_ids)
            .map_err(|e| DomainError::Storage(format!("serialize source_ids: {e}")))?;
        let success = rec.success as i64;
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO egress_log (id, created_at, provider, model, token_count, source_ids, success)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, created_at, provider, model, token_count, source_ids, success],
                )?;
                Ok(())
            })
            .await
    }

    async fn set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError> {
        let id = id.to_string();
        let grounded = grounded as i64;
        self.db
            .call(move |c| {
                let rows = c.execute(
                    "UPDATE egress_log SET grounded = ?2 WHERE id = ?1",
                    params![id, grounded],
                )?;
                if rows == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await
    }

    async fn list_recent(&self, limit: usize) -> Result<Vec<EgressRecord>, DomainError> {
        let limit = limit as i64;
        self.db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT id, created_at, provider, model, token_count, source_ids, success
                     FROM egress_log ORDER BY created_at DESC LIMIT ?1",
                )?;
                let rows = stmt
                    .query_map(params![limit], |r| {
                        let id: String = r.get(0)?;
                        let created_at: i64 = r.get(1)?;
                        let provider: String = r.get(2)?;
                        let model: String = r.get(3)?;
                        let token_count: i64 = r.get(4)?;
                        let source_ids_json: String = r.get(5)?;
                        let success: i64 = r.get(6)?;
                        let source_ids: Vec<SourceId> = serde_json::from_str(&source_ids_json)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        Ok(EgressRecord {
                            id: EgressLogId::parse(&id).map_err(|e| {
                                rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                            })?,
                            decision: EgressDecision {
                                provider,
                                model,
                                source_ids,
                                total_tokens: token_count as usize,
                            },
                            completed_at: created_at,
                            success: success != 0,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }
}

#[async_trait]
impl EgressSettings for SqliteEgressSettings {
    async fn consented(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare("SELECT provider FROM cloud_consent")?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<std::collections::HashSet<String>>>()?;
                Ok(rows)
            })
            .await
    }

    async fn grant(&self, provider: &str) -> Result<(), DomainError> {
        let provider = provider.to_string();
        // granted_at: a monotonic-ish stamp; the gate doesn't read it, so a constant is fine here.
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO cloud_consent (provider, granted_at) VALUES (?1, ?2)
                     ON CONFLICT(provider) DO NOTHING",
                    params![provider, 0_i64],
                )?;
                Ok(())
            })
            .await
    }

    async fn revoke(&self, provider: &str) -> Result<(), DomainError> {
        let provider = provider.to_string();
        self.db
            .call(move |c| {
                c.execute(
                    "DELETE FROM cloud_consent WHERE provider = ?1",
                    params![provider],
                )?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{EgressDecision, EgressLogId, SourceId};
    use std::collections::HashSet;

    fn rec() -> EgressRecord {
        EgressRecord {
            id: EgressLogId::new(),
            decision: EgressDecision {
                provider: "kimi".into(),
                model: "k2".into(),
                source_ids: vec![SourceId("n1".into()), SourceId("n2".into())],
                total_tokens: 42,
            },
            completed_at: 1000,
            success: true,
        }
    }

    #[tokio::test]
    async fn log_record_roundtrips_source_ids_json() {
        let db = Database::open_in_memory().unwrap();
        let log = SqliteEgressLog::new(db.clone());
        let r = rec();
        log.record(&r).await.unwrap();
        let (provider, ids_json, success): (String, String, i64) = db
            .call(move |c| {
                c.query_row(
                    "SELECT provider, source_ids, success FROM egress_log",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
            })
            .await
            .unwrap();
        assert_eq!(provider, "kimi");
        assert_eq!(success, 1);
        let ids: Vec<String> = serde_json::from_str(&ids_json).unwrap();
        assert_eq!(ids, vec!["n1".to_string(), "n2".to_string()]);
    }

    #[tokio::test]
    async fn set_grounded_updates_the_row() {
        let db = Database::open_in_memory().unwrap();
        let log = SqliteEgressLog::new(db.clone());
        let r = rec();
        let id = r.id;
        log.record(&r).await.unwrap();
        log.set_grounded(&id, false).await.unwrap();
        let grounded: Option<i64> = db
            .call(move |c| c.query_row("SELECT grounded FROM egress_log", [], |row| row.get(0)))
            .await
            .unwrap();
        assert_eq!(grounded, Some(0));
    }

    #[tokio::test]
    async fn settings_grant_and_revoke_consent() {
        let db = Database::open_in_memory().unwrap();
        let s = SqliteEgressSettings::new(db.clone());
        assert!(s.consented().await.unwrap().is_empty());
        s.grant("kimi").await.unwrap();
        assert_eq!(
            s.consented().await.unwrap(),
            HashSet::from(["kimi".to_string()])
        );
        s.revoke("kimi").await.unwrap();
        assert!(s.consented().await.unwrap().is_empty());
    }
}
