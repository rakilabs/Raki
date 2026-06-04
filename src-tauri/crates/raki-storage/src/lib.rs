//! SQLite-backed adapters implementing `raki-domain` ports. The only place SQL lives.

mod db;
mod migrations;
mod notes;
mod search;

pub use db::Database;
pub use notes::SqliteNoteRepository;
pub use search::SqliteKeywordIndex;
