import { createSignal, For, Show, createEffect } from "solid-js";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { notesApi, notesKeys } from "./api";

export function NotesView() {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal("");
  const [search, setSearch] = createSignal("");
  const [debouncedSearch, setDebouncedSearch] = createSignal("");

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

  const createNote = createMutation(() => ({
    mutationFn: () => notesApi.create({ title: title(), body: "{}" }),
    onSuccess: () => {
      setTitle("");
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
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

      <Show when={!notes.isLoading} fallback={<p>Loading…</p>}>
        <ul>
          <For each={notes.data ?? []}>{(n) => <li>{n.title}</li>}</For>
        </ul>
      </Show>
    </section>
  );
}
