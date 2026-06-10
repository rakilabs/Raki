import { commands, type CreateNoteInput, type UpdateNoteInput } from "~/shared/ipc";

export const notesKeys = {
  all: ["notes"] as const,
  search: (q: string) => ["notes", "search", q] as const,
  trashed: ["notes", "trashed"] as const,
};

export const notesApi = {
  list: () => commands.listNotes(),
  create: (input: CreateNoteInput) => commands.createNote(input),
  search: (query: string) => commands.searchNotes(query),
  update: (input: UpdateNoteInput) => commands.updateNote(input),
  delete: (id: string) => commands.deleteNote(id),
  restore: (id: string) => commands.restoreNote(id),
  listTrashed: () => commands.listTrashedNotes(),
};
