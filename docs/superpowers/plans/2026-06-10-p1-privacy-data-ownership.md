# P1 — Privacy & Data Ownership (minimal, Track-A-unblocking)

> **Scope discipline:** functional only; no UX polish. The goal is to make the app trustworthy enough to hold real notes, which unblocks Track A's real-data tier.

## Goal

Expose the already-built substrate (soft-delete, egress log, consent management) via thin Tauri commands and minimal frontend surfaces. No new infrastructure — only wiring and UI.

## What P1 unlocks

- Real notes can be deleted/trashed/restored → users trust the app with real content
- Egress settings are visible/controllable → users understand what leaves their device
- Audit log is viewable → "what left my device?" is answerable
- This enables dogfooding → real-notes eval tier (R1 verdict, R2 migration, R4 signals)

## Tasks

### Task 1: Backend commands for note lifecycle

**Files:**
- `src-tauri/src/commands/notes.rs` — add `delete_note`, `restore_note`, `list_trashed_notes`
- `src-tauri/src/dto.rs` — add `deleted_at: Option<i64>` to `NoteDto`
- `src-tauri/src/lib.rs` — register commands

**Details:**
- `delete_note(id: String) -> Result<(), AppError>` — calls `repo.soft_delete`, idempotent
- `restore_note(id: String) -> Result<NoteDto, AppError>` — sets `deleted_at = NULL`, reindexes FTS + vectors, bumps version
- `list_trashed_notes() -> Result<Vec<NoteDto>, AppError>` — lists notes where `deleted_at IS NOT NULL`
- `NoteDto` gains `deleted_at: Option<i64>` so frontend can render trash status

**Verification:** `cargo test --workspace --exclude raki` green; clippy clean.

### Task 2: Backend commands for egress settings & audit log

**Files:**
- `src-tauri/src/commands/settings.rs` — CREATE (or extend existing)
- `src-tauri/src/dto.rs` — add `EgressSettingsDto`, `EgressLogEntryDto`
- `src-tauri/src/lib.rs` — register commands

**Details:**
- `get_egress_settings() -> Result<EgressSettingsDto, AppError>` — reads mode + consented providers
- `set_egress_mode(mode: String) -> Result<(), AppError>` — `"local_only"` or `"cloud_allowed"`
- `grant_provider_consent(provider: String) -> Result<(), AppError>` — thin wrapper
- `revoke_provider_consent(provider: String) -> Result<(), AppError>` — thin wrapper
- `list_egress_log(limit: usize) -> Result<Vec<EgressLogEntryDto>, AppError>` — recent log entries newest-first

**Verification:** commands compile; no logic (thin adapters only).

### Task 3: Frontend — note delete + trash view

**Files:**
- `src/modules/notes/api.ts` — add `deleteNote`, `restoreNote`, `listTrashedNotes`
- `src/modules/notes/NotesView.tsx` — add delete button per note; trash toggle to show deleted notes with restore button

**Details:**
- Delete button on each note row → calls `deleteNote` + invalidates query
- "Show trash" toggle in the list header → switches between live and trashed notes
- Restore button on trashed notes → calls `restoreNote` + invalidates query
- No animations, no confirmation dialog (scope discipline — functional only)

**Verification:** `tsc --noEmit` clean; app builds.

### Task 4: Frontend — minimal Settings surface

**Files:**
- `src/modules/settings/api.ts` — CREATE — typed IPC wrappers for settings commands
- `src/modules/settings/SettingsPanel.tsx` — CREATE — minimal modal/panel
- `src/app/App.tsx` — add Settings toggle button

**Details:**
- Egress mode toggle: "Local only" / "Cloud allowed" (radio or select)
- Provider consent list: show consented providers + grant/revoke per provider
- Audit log: simple scrollable list of recent egress entries (provider, model, token count, timestamp, success)
- Triggered by a "Settings" button in the app shell (next to AskBox or in a corner)
- No routing, no persistent panel state across reloads — just a modal/dialog

**Verification:** `tsc --noEmit` clean; app builds.

## Out of scope (explicitly deferred)

- Confirmation dialogs for destructive actions
- Keyboard shortcuts
- Settings persistence beyond the backend (frontend state resets on reload)
- Styled/animated transitions
- Export/import of notes (already exists as Markdown export)
- Automatic feed to eval-data/real (manual export remains the path)

## Definition of Done

- [ ] `cargo test --workspace --exclude raki` green
- [ ] `cargo clippy --workspace --exclude raki --all-targets -- -D warnings` green
- [ ] `cargo fmt --check` green
- [ ] Frontend `tsc --noEmit` green
- [ ] Manual walkthrough: create note → delete note → see in trash → restore note → change egress mode → grant/revoke consent → view audit log
