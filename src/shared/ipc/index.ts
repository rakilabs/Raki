import { invoke } from "@tauri-apps/api/core";
import type { NoteDto } from "./bindings/NoteDto";
import type { CreateNoteInput } from "./bindings/CreateNoteInput";
import type { UpdateNoteInput } from "./bindings/UpdateNoteInput";
import type { AnswerOutcome } from "./bindings/AnswerOutcome";

export type { NoteDto, CreateNoteInput, UpdateNoteInput, AnswerOutcome };

/// The single typed surface over Tauri commands. Components never call invoke() directly.
export const commands = {
  createNote: (input: CreateNoteInput) => invoke<NoteDto>("create_note", { input }),
  listNotes: () => invoke<NoteDto[]>("list_notes"),
  getNote: (id: string) => invoke<NoteDto | null>("get_note", { id }),
  searchNotes: (query: string) => invoke<NoteDto[]>("search_notes", { query }),
  updateNote: (input: UpdateNoteInput) => invoke<NoteDto>("update_note", { input }),
  answerQuestion: (query: string) => invoke<AnswerOutcome>("answer_question", { query }),
  grantCloudConsent: (provider: string) => invoke<null>("grant_cloud_consent", { provider }),
  revokeCloudConsent: (provider: string) => invoke<null>("revoke_cloud_consent", { provider }),
};
