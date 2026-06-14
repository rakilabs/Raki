import { fireEvent, render, screen } from "@solidjs/testing-library";
import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  notesKeys: {
    all: ["notes"],
    search: (q: string) => ["notes", "search", q],
    trashed: ["notes", "trashed"],
  },
  notesApi: {
    list: vi.fn(),
    create: vi.fn(),
    search: vi.fn(),
    update: vi.fn(),
    delete: vi.fn(),
    restore: vi.fn(),
    listTrashed: vi.fn(),
    exportForEval: vi.fn(),
    recordView: vi.fn(),
    getNote: vi.fn(),
  },
}));

vi.mock("./components/NoteEditor", () => ({
  NoteEditor: (props: { noteId: string; onDeleted: () => void }) => (
    <div data-testid="note-editor" data-note-id={props.noteId}>
      Note editor for {props.noteId}
    </div>
  ),
}));

import { notesApi } from "./api";
import { NotesView } from "./NotesView";

const mocked = vi.mocked(notesApi);

function renderView() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(() => (
    <QueryClientProvider client={client}>
      <NotesView />
    </QueryClientProvider>
  ));
}

describe("NotesView", () => {
  beforeEach(() => vi.clearAllMocks());

  it("selecting a note renders the editor for that note", async () => {
    mocked.list.mockResolvedValue([
      {
        id: "n1",
        title: "Trip",
        body: "{}",
        body_text: "Pay cash",
        created_at: 0,
        updated_at: 0,
        deleted_at: null,
      },
    ]);
    renderView();

    fireEvent.click(await screen.findByRole("button", { name: "Trip" }));
    const editor = await screen.findByTestId("note-editor");
    expect(editor).toHaveAttribute("data-note-id", "n1");
  });

  it("renders (Untitled) for a blank-title note", async () => {
    mocked.list.mockResolvedValue([
      {
        id: "n2",
        title: "  ",
        body: "{}",
        body_text: "",
        created_at: 0,
        updated_at: 0,
        deleted_at: null,
      },
    ]);
    renderView();
    expect(
      await screen.findByRole("button", { name: "(Untitled)" })
    ).toBeDefined();
  });
});
