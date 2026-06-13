//! One-off dev helper: import Markdown notes from a directory into the running Raki app's
//! SQLite database, then export all live notes to `eval-data/real/notes/` for the real-data eval
//! harness. Uses the backend storage API directly (no Tauri runtime).
//!
//! Usage:
//!   cargo run -p raki-eval --bin import-lifeos -- \
//!     --source /Users/jayden/Areas/LifeOS \
//!     --app-data ~/Library/Application\ Support/com.jayden.raki

use std::path::{Path, PathBuf};

use raki_domain::{text_to_body, Note, NoteRepository};
use raki_storage::{Database, SqliteNoteRepository};

#[derive(Debug)]
struct Args {
    source: PathBuf,
    app_data: PathBuf,
    clear_existing: bool,
}

fn parse_args() -> Args {
    let mut source = None;
    let mut app_data = None;
    let mut clear_existing = false;
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--source" => source = iter.next().map(PathBuf::from),
            "--app-data" => app_data = iter.next().map(PathBuf::from),
            "--clear-existing" => clear_existing = true,
            _ => {}
        }
    }
    Args {
        source: source.expect("--source <dir> required"),
        app_data: app_data.expect("--app-data <dir> required"),
        clear_existing,
    }
}

fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut prev_dash = true;
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

/// Recursively collect all `.md` files under `dir`, sorted for determinism.
fn collect_md(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_md(&path));
            } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();
    let db_path = args.app_data.join("raki.sqlite");
    if !db_path.exists() {
        std::fs::create_dir_all(&args.app_data)?;
    }

    let db = Database::open(&db_path)?;
    let notes_repo = SqliteNoteRepository::new(db.clone());

    if args.clear_existing {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let existing = notes_repo.list().await?;
        eprintln!("clearing {} existing live notes", existing.len());
        for note in existing {
            notes_repo.soft_delete(&note.id, now).await?;
        }
    }

    let md_files = collect_md(&args.source);
    eprintln!("found {} markdown files", md_files.len());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;

    for path in &md_files {
        let raw = std::fs::read_to_string(path)?;
        let stripped = raki_eval::markdown::strip_frontmatter(&raw);
        let title = raki_eval::markdown::first_h1(stripped)
            .unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().into_owned());
        let body_text = raki_eval::markdown::to_plain_text(stripped);
        let note = Note::new(title, text_to_body(&body_text), now);
        notes_repo.upsert(&note).await?;
    }

    // Export all live notes to eval-data/real/notes/ using the same shape as the Tauri command.
    let eval_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("eval-data")
        .join("real");
    let notes_dir = eval_dir.join("notes");
    std::fs::create_dir_all(&notes_dir)?;

    let live = notes_repo.list().await?;
    let exported = live.len();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for note in &live {
        let base = slugify(&note.title);
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        let slug = if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        };
        let path = notes_dir.join(format!("{slug}.md"));
        let body_text = raki_domain::body_to_text(&note.body);
        let content = format!(
            "---\ntitle: {}\nid: {}\n---\n\n# {}\n\n{}\n",
            note.title, note.id, note.title, body_text
        );
        std::fs::write(&path, content)?;
    }

    eprintln!(
        "imported {} notes into app DB and exported {} live notes to {}",
        md_files.len(),
        exported,
        notes_dir.display()
    );
    eprintln!(
        "next: author eval-data/real/queries.json, then run cargo run -p raki-eval --bin real-eval"
    );
    Ok(())
}
