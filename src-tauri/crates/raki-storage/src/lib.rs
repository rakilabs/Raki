//! SQLite-backed adapters implementing `raki-domain` ports. The only place SQL lives.

mod db;
mod hash;
mod indexing;
mod migrations;
mod notes;
mod search;
mod vectors;

pub use db::Database;
pub use indexing::SqliteIndexingStore;
pub use notes::SqliteNoteRepository;
pub use search::SqliteKeywordIndex;
pub use vectors::SqliteVectorIndex;
