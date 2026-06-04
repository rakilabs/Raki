import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("~/shared/ipc", () => ({
  commands: {
    searchNotes: vi.fn(),
    listNotes: vi.fn(),
    createNote: vi.fn(),
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
});
