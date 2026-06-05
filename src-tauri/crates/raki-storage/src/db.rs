//! A cloneable database handle. SQLite is single-writer, so for now all access is
//! serialized through one connection on a blocking thread. (Reader-pool is a later upgrade.)

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::ffi::sqlite3_auto_extension;
use rusqlite::Connection;
use sqlite_vec::sqlite3_vec_init;

use raki_domain::DomainError;

use crate::migrations::migrate;

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DomainError> {
        register_sqlite_vec();
        let conn = Connection::open(path).map_err(storage_err)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, DomainError> {
        register_sqlite_vec();
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

/// Register sqlite-vec as an auto-extension exactly once, before any connection
/// opens. `sqlite3_auto_extension` applies to every connection opened afterward.
#[allow(clippy::missing_transmute_annotations)]
fn register_sqlite_vec() {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    });
}

pub(crate) fn storage_err(e: rusqlite::Error) -> DomainError {
    DomainError::Storage(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_vec_extension_is_registered() {
        let db = Database::open_in_memory().unwrap();
        // vec_version() only resolves if the sqlite-vec extension loaded.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let version: String = rt.block_on(async {
            db.call(|c| c.query_row("SELECT vec_version()", [], |r| r.get(0)))
                .await
                .unwrap()
        });
        assert!(version.starts_with('v'), "got vec_version = {version}");
    }
}
