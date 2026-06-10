import { invoke } from "@tauri-apps/api/core";
import type { NoteDto } from "./bindings/NoteDto";
import type { CreateNoteInput } from "./bindings/CreateNoteInput";
import type { UpdateNoteInput } from "./bindings/UpdateNoteInput";
import type { AnswerOutcome } from "./bindings/AnswerOutcome";
import type { EgressSettingsDto } from "./bindings/EgressSettingsDto";
import type { EgressLogEntryDto } from "./bindings/EgressLogEntryDto";

export type { NoteDto, CreateNoteInput, UpdateNoteInput, AnswerOutcome, EgressSettingsDto, EgressLogEntryDto };

/// The single typed surface over Tauri commands. Components never call invoke() directly.
export const commands = {
  createNote: (input: CreateNoteInput) => invoke<NoteDto>("create_note", { input }),
  listNotes: () => invoke<NoteDto[]>("list_notes"),
  getNote: (id: string) => invoke<NoteDto | null>("get_note", { id }),
  searchNotes: (query: string) => invoke<NoteDto[]>("search_notes", { query }),
  updateNote: (input: UpdateNoteInput) => invoke<NoteDto>("update_note", { input }),
  deleteNote: (id: string) => invoke<null>("delete_note", { id }),
  restoreNote: (id: string) => invoke<NoteDto>("restore_note", { id }),
  listTrashedNotes: () => invoke<NoteDto[]>("list_trashed_notes"),
  answerQuestion: (query: string) => invoke<AnswerOutcome>("answer_question", { query }),
  getEgressSettings: () => invoke<EgressSettingsDto>("get_egress_settings"),
  setEgressMode: (mode: string) => invoke<null>("set_egress_mode", { mode }),
  grantProviderConsent: (provider: string) => invoke<null>("grant_provider_consent", { provider }),
  revokeProviderConsent: (provider: string) => invoke<null>("revoke_provider_consent", { provider }),
  listEgressLog: (limit: number) => invoke<EgressLogEntryDto[]>("list_egress_log", { limit }),
};
