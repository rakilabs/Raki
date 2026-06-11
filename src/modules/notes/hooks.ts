import { createQuery } from "@tanstack/solid-query";
import type { Accessor } from "solid-js";
import { notesApi, notesKeys } from "./api";

export function useNotes(opts: {
  search: Accessor<string>;
  showTrash: Accessor<boolean>;
}) {
  return createQuery(() => {
    const q = opts.search().trim();
    const trash = opts.showTrash();
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
}
