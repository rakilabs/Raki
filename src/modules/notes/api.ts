import {
  type CreateNoteInput,
  commands,
  type UpdateNoteInput,
} from "~/shared/ipc";

export const notesKeys = {
  all: ["notes"] as const,
  search: (q: string) => ["notes", "search", q] as const,
  trashed: ["notes", "trashed"] as const,
};

export const notesApi = {
  list: () => commands.listNotes(),
  getNote: (id: string) => commands.getNote(id),
  create: (input: CreateNoteInput) => commands.createNote(input),
  search: (query: string) => commands.searchNotes(query),
  update: (input: UpdateNoteInput) => commands.updateNote(input),
  delete: (id: string) => commands.deleteNote(id),
  restore: (id: string) => commands.restoreNote(id),
  listTrashed: () => commands.listTrashedNotes(),
};
