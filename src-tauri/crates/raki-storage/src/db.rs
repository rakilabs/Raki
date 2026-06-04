//! A cloneable database handle. SQLite is single-writer, so for now all access is
//! serialized through one connection on a blocking thread. (Reader-pool is a later upgrade.)

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use raki_domain::DomainError;

use crate::migrations::migrate;

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DomainError> {
        let conn = Connection::open(path).map_err(storage_err)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, DomainError> {
        let conn = Connection::open_in_memory().map_err(storage_err)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, DomainError> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(storage_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(storage_err)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(storage_err)?;
        migrate(&conn).map_err(storage_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure with the connection on a blocking thread.
    pub async fn call<F, T>(&self, f: F) -> Result<T, DomainError>
    where
        F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().expect("db mutex poisoned");
            f(&guard).map_err(storage_err)
        })
        .await
        .map_err(|e| DomainError::Storage(format!("join error: {e}")))?
    }
}

pub(crate) fn storage_err(e: rusqlite::Error) -> DomainError {
    DomainError::Storage(e.to_string())
}
