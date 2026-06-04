import { invoke } from "@tauri-apps/api/core";
import type { NoteDto } from "./bindings/NoteDto";
import type { CreateNoteInput } from "./bindings/CreateNoteInput";

export type { NoteDto, CreateNoteInput };

/// The single typed surface over Tauri commands. Components never call invoke() directly.
export const commands = {
  createNote: (input: CreateNoteInput) => invoke<NoteDto>("create_note", { input }),
  listNotes: () => invoke<NoteDto[]>("list_notes"),
  getNote: (id: string) => invoke<NoteDto | null>("get_note", { id }),
  searchNotes: (query: string) => invoke<NoteDto[]>("search_notes", { query }),
};
