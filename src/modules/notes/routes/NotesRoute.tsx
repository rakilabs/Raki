import { useSearchParams } from "@solidjs/router";
import { createQuery, useQueryClient } from "@tanstack/solid-query";
import { createSignal, Show } from "solid-js";
import { notesApi, notesKeys } from "~/modules/notes/api";
import { NoteCreateForm } from "~/modules/notes/components/NoteCreateForm";
import { NoteEditor } from "~/modules/notes/components/NoteEditor";
import { NoteList } from "~/modules/notes/components/NoteList";
import { Card } from "~/shared/ui";

export default function NotesRoute() {
  const [searchParams, setSearchParams] = useSearchParams();
  const queryClient = useQueryClient();
  const selectedId = () => {
    const id = searchParams.id;
    return typeof id === "string" ? id : null;
  };
  const [showTrash, setShowTrash] = createSignal(false);

  const notes = createQuery(() => ({
    queryKey: showTrash() ? notesKeys.trashed : notesKeys.all,
    queryFn: () => (showTrash() ? notesApi.listTrashed() : notesApi.list()),
  }));

  const handleSelect = (id: string | null) => {
    setSearchParams({ id: id || undefined });
  };

  return (
    <div class="flex h-full gap-4 p-4">
      {/* Sidebar */}
      <div class="flex w-80 flex-col gap-3">
        <NoteCreateForm
          onCreated={(id) => {
            queryClient.invalidateQueries({ queryKey: notesKeys.all });
            handleSelect(id);
          }}
        />
        <Card class="flex-1 overflow-hidden">
          <NoteList
            notes={notes.data ?? []}
            selectedId={selectedId()}
            onSelect={handleSelect}
            showTrash={showTrash()}
            onToggleTrash={() => {
              setShowTrash((v) => !v);
              handleSelect(null);
            }}
            isLoading={notes.isLoading}
          />
        </Card>
      </div>

      {/* Editor */}
      <div class="flex-1">
        <Show
          when={selectedId()}
          fallback={
            <Card class="flex h-full items-center justify-center text-muted-foreground">
              <p>Select a note to edit</p>
            </Card>
          }
        >
          <Show when={selectedId()}>
            {(id) => (
              <NoteEditor
                noteId={id()}
                onDeleted={() => {
                  queryClient.invalidateQueries({ queryKey: notesKeys.all });
                  handleSelect(null);
                }}
              />
            )}
          </Show>
        </Show>
      </div>
    </div>
  );
}
