import { Editor } from "@tiptap/core";
import Placeholder from "@tiptap/extension-placeholder";
import StarterKit from "@tiptap/starter-kit";
import { createEffect, createSignal, onCleanup } from "solid-js";
import { BlockId } from "./BlockId";
import "./TipTapEditor.css";

interface TipTapEditorProps {
  bodyJson: string;
  onChange: (bodyJson: string) => void;
  placeholder?: string;
}

export function TipTapEditor(props: TipTapEditorProps) {
  let mountRef: HTMLDivElement | undefined;
  const [editor, setEditor] = createSignal<Editor | null>(null);

  createEffect(() => {
    const ed = new Editor({
      element: mountRef,
      extensions: [
        StarterKit,
        Placeholder.configure({
          placeholder: props.placeholder ?? "Start writing...",
        }),
        BlockId,
      ],
      content: props.bodyJson,
      autofocus: false,
      onUpdate: ({ editor }) => {
        props.onChange(JSON.stringify(editor.state.doc.toJSON()));
      },
    });
    setEditor(ed);
    onCleanup(() => ed.destroy());
  });

  // Reset content when the external note changes, but preserve editor focus/selection where possible.
  createEffect((prevJson?: string) => {
    const ed = editor();
    if (!ed) return props.bodyJson;
    const json = props.bodyJson;
    if (json === prevJson) return json;
    const current = JSON.stringify(ed.state.doc.toJSON());
    if (current !== json) {
      ed.commands.setContent(json, false);
    }
    return json;
  });

  return (
    <div
      ref={mountRef}
      class="min-h-[200px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-within:ring-2 focus-within:ring-ring"
    />
  );
}
