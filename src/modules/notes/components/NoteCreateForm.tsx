import { createMutation, useQueryClient } from "@tanstack/solid-query";
import { Plus } from "lucide-solid";
import { createSignal } from "solid-js";
import { notesApi, notesKeys } from "~/modules/notes/api";
import { Button, Input } from "~/shared/ui";

interface NoteCreateFormProps {
  onCreated: (id: string) => void;
}

export function NoteCreateForm(props: NoteCreateFormProps) {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal("");

  const createNote = createMutation(() => ({
    mutationFn: () => notesApi.create({ title: title(), body: "" }),
    onSuccess: (note) => {
      setTitle("");
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
      props.onCreated(note.id);
    },
  }));

  const handleSubmit = (e: Event) => {
    e.preventDefault();
    if (title().trim()) createNote.mutate();
  };

  return (
    <form onSubmit={handleSubmit} class="flex gap-2">
      <Input
        class="flex-1"
        placeholder="New note title..."
        value={title()}
        onInput={(e) => setTitle(e.currentTarget.value)}
      />
      <Button
        type="submit"
        disabled={!title().trim() || createNote.isPending}
        loading={createNote.isPending}
      >
        <Plus class="h-4 w-4" />
        <span class="hidden sm:inline">Add</span>
      </Button>
    </form>
  );
}
