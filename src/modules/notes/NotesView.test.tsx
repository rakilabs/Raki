import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, screen, waitFor } from "@solidjs/testing-library";
import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";

vi.mock("./api", () => ({
  notesKeys: { all: ["notes"], search: (q: string) => ["notes", "search", q] },
  notesApi: { list: vi.fn(), create: vi.fn(), search: vi.fn(), update: vi.fn() },
}));

import { notesApi } from "./api";
import { NotesView } from "./NotesView";

const mocked = vi.mocked(notesApi);

function renderView() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(() => (
    <QueryClientProvider client={client}>
      <NotesView />
    </QueryClientProvider>
  ));
}

describe("NotesView editor", () => {
  beforeEach(() => vi.clearAllMocks());

  it("selecting a note populates the editor and Save delegates to update", async () => {
    mocked.list.mockResolvedValue([
      { id: "n1", title: "Trip", body: "Pay cash", created_at: 0, updated_at: 0 },
    ]);
    mocked.update.mockResolvedValue({ id: "n1", title: "Trip", body: "Pay card", created_at: 0, updated_at: 1 });
    renderView();

    fireEvent.click(await screen.findByRole("button", { name: "Trip" }));
    const body = (await screen.findByLabelText("Body")) as HTMLTextAreaElement;
    expect(body.value).toBe("Pay cash");

    fireEvent.input(body, { target: { value: "Pay card" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocked.update).toHaveBeenCalledWith({ id: "n1", title: "Trip", body: "Pay card" }),
    );
  });

  it("renders (Untitled) for a blank-title note", async () => {
    mocked.list.mockResolvedValue([{ id: "n2", title: "  ", body: "", created_at: 0, updated_at: 0 }]);
    renderView();
    expect(await screen.findByRole("button", { name: "(Untitled)" })).toBeDefined();
  });
});
