import { createSignal, createEffect, createMemo, For, Show } from "solid-js";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { notesApi, notesKeys } from "./api";

export function NotesView() {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal("");
  const [search, setSearch] = createSignal("");
  const [debouncedSearch, setDebouncedSearch] = createSignal("");
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [editTitle, setEditTitle] = createSignal("");
  const [editBody, setEditBody] = createSignal("");

  createEffect(() => {
    const q = search();
    const timer = setTimeout(() => setDebouncedSearch(q.trim()), 150);
    return () => clearTimeout(timer);
  });

  const notes = createQuery(() => {
    const q = debouncedSearch();
    return {
      queryKey: q ? notesKeys.search(q) : notesKeys.all,
      queryFn: () => (q ? notesApi.search(q) : notesApi.list()),
    };
  });

  const selected = createMemo(() =>
    (notes.data ?? []).find((n) => n.id === selectedId()),
  );

  // Seed the editor fields only when the selected note id changes (not on every query refresh).
  createEffect(() => {
    const id = selectedId();
    if (id) {
      const n = (notes.data ?? []).find((n) => n.id === id);
      if (n) {
        setEditTitle(n.title);
        setEditBody(n.body);
      }
    }
  });

  const createNote = createMutation(() => ({
    mutationFn: () => notesApi.create({ title: title(), body: "" }),
    onSuccess: () => {
      setTitle("");
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
    },
  }));

  const saveNote = createMutation(() => ({
    mutationFn: (vars: { id: string; title: string; body: string }) =>
      notesApi.update(vars),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: notesKeys.all }),
  }));

  return (
    <section>
      <h1>Notes</h1>

      <input
        type="search"
        placeholder="Search notes…"
        value={search()}
        onInput={(e) => setSearch(e.currentTarget.value)}
      />

      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (title().trim()) createNote.mutate();
        }}
      >
        <input
          placeholder="New note title…"
          value={title()}
          onInput={(e) => setTitle(e.currentTarget.value)}
        />
        <button type="submit" disabled={createNote.isPending}>
          Add
        </button>
      </form>

      <div class="notes-layout">
        <Show when={!notes.isLoading} fallback={<p>Loading…</p>}>
          <ul>
            <For each={notes.data ?? []}>
              {(n) => (
                <li>
                  <button type="button" onClick={() => setSelectedId(n.id)}>
                    {n.title.trim() || "(Untitled)"}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>

        <Show when={selected()}>
          {(s) => (
            <form
              class="note-editor"
              onSubmit={(e) => {
                e.preventDefault();
                const n = s();
                if (n && editTitle().trim()) {
                  saveNote.mutate({
                    id: n.id,
                    title: editTitle(),
                    body: editBody(),
                  });
                }
              }}
            >
              <input
                aria-label="Title"
                value={editTitle()}
                onInput={(e) => setEditTitle(e.currentTarget.value)}
              />
              <textarea
                aria-label="Body"
                value={editBody()}
                onInput={(e) => setEditBody(e.currentTarget.value)}
              />
              <button
                type="submit"
                disabled={saveNote.isPending || !editTitle().trim()}
              >
                Save
              </button>
              <Show when={saveNote.isError}>
                <p role="alert">Save failed — please try again.</p>
              </Show>
            </form>
          )}
        </Show>
      </div>
    </section>
  );
}
