import {
  createMutation,
  createQuery,
  useQueryClient,
} from "@tanstack/solid-query";
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { notesApi, notesKeys } from "./api";

export function NotesView() {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal("");
  const [search, setSearch] = createSignal("");
  const [debouncedSearch, setDebouncedSearch] = createSignal("");
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [editTitle, setEditTitle] = createSignal("");
  const [editBody, setEditBody] = createSignal("");
  const [showTrash, setShowTrash] = createSignal(false);
  const [exportMessage, setExportMessage] = createSignal<string | null>(null);

  createEffect(() => {
    const q = search();
    const timer = setTimeout(() => setDebouncedSearch(q.trim()), 150);
    return () => clearTimeout(timer);
  });

  const notes = createQuery(() => {
    const q = debouncedSearch();
    const trash = showTrash();
    return {
      queryKey: trash
        ? notesKeys.trashed
        : q
          ? notesKeys.search(q)
          : notesKeys.all,
      queryFn: () =>
        trash
          ? notesApi.listTrashed()
          : q
            ? notesApi.search(q)
            : notesApi.list(),
    };
  });

  const selected = createMemo(() =>
    (notes.data ?? []).find((n) => n.id === selectedId())
  );

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

  const deleteNote = createMutation(() => ({
    mutationFn: (id: string) => notesApi.delete(id),
    onSuccess: () => {
      setSelectedId(null);
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
      queryClient.invalidateQueries({ queryKey: notesKeys.trashed });
    },
  }));

  const restoreNote = createMutation(() => ({
    mutationFn: (id: string) => notesApi.restore(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
      queryClient.invalidateQueries({ queryKey: notesKeys.trashed });
    },
  }));

  const exportForEval = createMutation(() => ({
    mutationFn: () => notesApi.exportForEval(),
    onSuccess: (res) => {
      setExportMessage(
        `Exported ${res.exported} note(s) to eval-data/real/notes/`
      );
      setTimeout(() => setExportMessage(null), 4000);
    },
    onError: () => {
      setExportMessage("Export failed — see backend logs.");
      setTimeout(() => setExportMessage(null), 4000);
    },
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

      <Show when={!showTrash()}>
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
      </Show>

      <label>
        <input
          type="checkbox"
          checked={showTrash()}
          onChange={(e) => {
            setShowTrash(e.currentTarget.checked);
            setSelectedId(null);
          }}
        />
        Show trash
      </label>

      <button
        type="button"
        onClick={() => exportForEval.mutate()}
        disabled={exportForEval.isPending || showTrash()}
        title="Export live notes to eval-data/real/notes/ for local eval"
      >
        Export for eval
      </button>
      <Show when={exportMessage()}>
        {(msg) => <p role="status">{msg()}</p>}
      </Show>

      <div class="notes-layout">
        <Show when={!notes.isLoading} fallback={<p>Loading…</p>}>
          <ul>
            <For each={notes.data ?? []}>
              {(n) => (
                <li>
                  <button type="button" onClick={() => setSelectedId(n.id)}>
                    {n.title.trim() || "(Untitled)"}
                  </button>
                  <Show
                    when={showTrash()}
                    fallback={
                      <button
                        type="button"
                        onClick={() => deleteNote.mutate(n.id)}
                        disabled={deleteNote.isPending}
                        title="Delete"
                      >
                        🗑
                      </button>
                    }
                  >
                    <button
                      type="button"
                      onClick={() => restoreNote.mutate(n.id)}
                      disabled={restoreNote.isPending}
                      title="Restore"
                    >
                      ↩
                    </button>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>

        <Show when={!showTrash() && selected()}>
          {(s) => {
            const n = s();
            return (
              <form
                class="note-editor"
                onSubmit={(e) => {
                  e.preventDefault();
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
            );
          }}
        </Show>
      </div>
    </section>
  );
}
