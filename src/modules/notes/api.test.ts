import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("~/shared/ipc", () => ({
  commands: {
    searchNotes: vi.fn(),
    listNotes: vi.fn(),
    createNote: vi.fn(),
    updateNote: vi.fn(),
  },
}));

import { commands } from "~/shared/ipc";
import { notesApi } from "./api";

const mocked = vi.mocked(commands);

describe("notesApi", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("search delegates to the searchNotes command with the query", async () => {
    mocked.searchNotes.mockResolvedValue([]);
    await notesApi.search("apples");
    expect(mocked.searchNotes).toHaveBeenCalledWith("apples");
  });

  it("update delegates to the updateNote command with the input", async () => {
    mocked.updateNote.mockResolvedValue({ id: "n1", title: "t", body: "b", created_at: 0, updated_at: 1 });
    await notesApi.update({ id: "n1", title: "t", body: "b" });
    expect(mocked.updateNote).toHaveBeenCalledWith({ id: "n1", title: "t", body: "b" });
  });
});
