import { createMutation, useQueryClient } from "@tanstack/solid-query";
import { FileText, RotateCcw, Trash2 } from "lucide-solid";
import { For, Show } from "solid-js";
import { notesApi, notesKeys } from "~/modules/notes/api";
import type { NoteDto } from "~/shared/ipc";
import { cn } from "~/shared/lib/cn";
import { Badge, Button, Skeleton } from "~/shared/ui";

interface NoteListProps {
  notes: NoteDto[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  showTrash: boolean;
  onToggleTrash: () => void;
  isLoading: boolean;
}

export function NoteList(props: NoteListProps) {
  const queryClient = useQueryClient();

  const deleteNote = createMutation(() => ({
    mutationFn: (id: string) => notesApi.delete(id),
    onSuccess: () => {
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

  return (
    <div class="flex h-full flex-col">
      <div class="flex items-center justify-between border-b border-border p-3">
        <h2 class="font-semibold text-sm">
          {props.showTrash ? "Trash" : "Notes"}
          <Show when={props.notes.length > 0}>
            <Badge variant="secondary" class="ml-2">
              {props.notes.length}
            </Badge>
          </Show>
        </h2>
        <Button variant="ghost" size="sm" onClick={props.onToggleTrash}>
          {props.showTrash ? "Back to Notes" : "Trash"}
        </Button>
      </div>

      <div class="flex-1 overflow-auto p-2">
        <Show when={props.isLoading}>
          <div class="space-y-2 p-2">
            <Skeleton height="48px" />
            <Skeleton height="48px" />
            <Skeleton height="48px" />
          </div>
        </Show>

        <Show when={!props.isLoading && props.notes.length === 0}>
          <div class="flex flex-col items-center justify-center py-12 text-muted-foreground">
            <FileText class="mb-2 h-8 w-8 opacity-50" />
            <p class="text-sm">
              {props.showTrash ? "Trash is empty" : "No notes yet"}
            </p>
          </div>
        </Show>

        <For each={props.notes}>
          {(note) => (
            <button
              type="button"
              class={cn(
                "group flex w-full items-center gap-2 rounded-md px-3 py-2.5 text-sm text-left transition-colors",
                props.selectedId === note.id
                  ? "bg-primary-50 text-primary-900 dark:bg-primary-950 dark:text-primary-200"
                  : "hover:bg-muted"
              )}
              onClick={() => props.onSelect(note.id)}
            >
              <span class="flex-1 truncate font-medium">
                {note.title.trim() || "(Untitled)"}
              </span>
              <Show when={!props.showTrash}>
                <Button
                  variant="ghost"
                  size="icon"
                  class="h-7 w-7 opacity-0 group-hover:opacity-100"
                  onClick={(e) => {
                    e.stopPropagation();
                    deleteNote.mutate(note.id);
                  }}
                  disabled={deleteNote.isPending}
                >
                  <Trash2 class="h-3.5 w-3.5 text-muted-foreground" />
                </Button>
              </Show>
              <Show when={props.showTrash}>
                <Button
                  variant="ghost"
                  size="icon"
                  class="h-7 w-7"
                  onClick={(e) => {
                    e.stopPropagation();
                    restoreNote.mutate(note.id);
                  }}
                  disabled={restoreNote.isPending}
                >
                  <RotateCcw class="h-3.5 w-3.5 text-muted-foreground" />
                </Button>
              </Show>
            </button>
          )}
        </For>
      </div>
    </div>
  );
}
