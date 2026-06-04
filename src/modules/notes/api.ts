import { commands, type CreateNoteInput } from "~/shared/ipc";

export const notesKeys = {
  all: ["notes"] as const,
};

export const notesApi = {
  list: () => commands.listNotes(),
  create: (input: CreateNoteInput) => commands.createNote(input),
};
