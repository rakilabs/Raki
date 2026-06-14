import {
  createMutation,
  createQuery,
  useQueryClient,
} from "@tanstack/solid-query";
import { Save, Trash2 } from "lucide-solid";
import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import { notesApi, notesKeys } from "~/modules/notes/api";
import {
  Button,
  Card,
  CardContent,
  Input,
  Spinner,
  useToast,
} from "~/shared/ui";
import { TipTapEditor } from "./TipTapEditor";

interface NoteEditorProps {
  noteId: string;
  onDeleted: () => void;
}

// Per-note debounce: don't record another view within 5 s of the previous one.
const lastRecordedView = new Map<string, number>();

export function NoteEditor(props: NoteEditorProps) {
  const queryClient = useQueryClient();
  const toast = useToast();
  const [title, setTitle] = createSignal("");
  const [bodyJson, setBodyJson] = createSignal("");

  // Record a view after the note has been active for 2 s. The backend rate-limits
  // to one increment per note per minute; the frontend avoids spamming calls.
  createEffect(() => {
    const noteId = props.noteId;
    const now = Date.now();
    if (now - (lastRecordedView.get(noteId) ?? 0) < 5000) {
      return;
    }

    const timer = setTimeout(() => {
      notesApi.recordView(noteId).catch(() => {
        // View counting is best-effort; don't disturb the editor on failure.
      });
      lastRecordedView.set(noteId, Date.now());
    }, 2000);

    onCleanup(() => clearTimeout(timer));
  });

  const note = createQuery(() => ({
    queryKey: ["note", props.noteId],
    queryFn: () => notesApi.getNote(props.noteId),
  }));

  createEffect(() => {
    const n = note.data;
    if (n) {
      setTitle(n.title);
      setBodyJson(n.body);
    }
  });

  const saveNote = createMutation(() => ({
    mutationFn: () =>
      notesApi.update({
        id: props.noteId,
        title: title(),
        body: bodyJson(),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
      toast.add({ type: "success", message: "Note saved" });
    },
    onError: () => {
      toast.add({ type: "error", message: "Failed to save note" });
    },
  }));

  const deleteNote = createMutation(() => ({
    mutationFn: () => notesApi.delete(props.noteId),
    onSuccess: () => {
      toast.add({ type: "success", message: "Note moved to trash" });
      props.onDeleted();
    },
  }));

  const isDirty = () => {
    const n = note.data;
    if (!n) return false;
    return n.title !== title() || n.body !== bodyJson();
  };

  return (
    <Card class="h-full">
      <Show when={note.isLoading}>
        <div class="flex h-full items-center justify-center">
          <Spinner />
        </div>
      </Show>

      <Show when={note.data}>
        <CardContent class="flex h-full flex-col gap-4 pt-6">
          <div class="flex items-center gap-2">
            <Input
              class="flex-1"
              value={title()}
              onInput={(e) => setTitle(e.currentTarget.value)}
              placeholder="Note title..."
            />
            <Button
              onClick={() => saveNote.mutate()}
              disabled={!isDirty() || saveNote.isPending}
              loading={saveNote.isPending}
            >
              <Save class="h-4 w-4" />
              Save
            </Button>
            <Button
              variant="destructive"
              size="icon"
              onClick={() => deleteNote.mutate()}
              loading={deleteNote.isPending}
            >
              <Trash2 class="h-4 w-4" />
            </Button>
          </div>

          <TipTapEditor
            bodyJson={bodyJson()}
            onChange={setBodyJson}
            placeholder="Start writing..."
          />

          <Show when={note.data}>
            {(n) => (
              <p class="text-xs text-muted-foreground">
                Last updated: {new Date(n().updated_at).toLocaleString()}
              </p>
            )}
          </Show>
        </CardContent>
      </Show>
    </Card>
  );
}
