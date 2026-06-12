import { invoke } from "@tauri-apps/api/core";
import type { AnswerOutcome } from "./bindings/AnswerOutcome";
import type { CitedNote } from "./bindings/CitedNote";
import type { CreateNoteInput } from "./bindings/CreateNoteInput";
import type { EgressLogEntryDto } from "./bindings/EgressLogEntryDto";
import type { EgressPreviewDto } from "./bindings/EgressPreviewDto";
import type { EgressSettingsDto } from "./bindings/EgressSettingsDto";
import type { NoteDto } from "./bindings/NoteDto";
import type { RecordNoteViewInput } from "./bindings/RecordNoteViewInput";
import type { UpdateNoteInput } from "./bindings/UpdateNoteInput";

export type {
  AnswerOutcome,
  CitedNote,
  CreateNoteInput,
  EgressLogEntryDto,
  EgressPreviewDto,
  EgressSettingsDto,
  NoteDto,
  RecordNoteViewInput,
  UpdateNoteInput,
};

/// The single typed surface over Tauri commands. Components never call invoke() directly.
export const commands = {
  createNote: (input: CreateNoteInput) =>
    invoke<NoteDto>("create_note", { input }),
  listNotes: () => invoke<NoteDto[]>("list_notes"),
  recordNoteView: (input: RecordNoteViewInput) =>
    invoke<null>("record_note_view", { input }),
  getNote: (id: string) => invoke<NoteDto | null>("get_note", { id }),
  searchNotes: (query: string) => invoke<NoteDto[]>("search_notes", { query }),
  updateNote: (input: UpdateNoteInput) =>
    invoke<NoteDto>("update_note", { input }),
  deleteNote: (id: string) => invoke<null>("delete_note", { id }),
  restoreNote: (id: string) => invoke<NoteDto>("restore_note", { id }),
  listTrashedNotes: () => invoke<NoteDto[]>("list_trashed_notes"),
  answerQuestion: (query: string) =>
    invoke<AnswerOutcome>("answer_question", { query }),
  getEgressSettings: () => invoke<EgressSettingsDto>("get_egress_settings"),
  grantProviderConsent: (provider: string) =>
    invoke<null>("grant_provider_consent", { provider }),
  revokeProviderConsent: (provider: string) =>
    invoke<null>("revoke_provider_consent", { provider }),
  listEgressLog: (limit: number) =>
    invoke<EgressLogEntryDto[]>("list_egress_log", { limit }),
};
