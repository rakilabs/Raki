import { Extension } from "@tiptap/core";
import { v7 as uuidv7 } from "uuid";

function newBlockId(): string {
  return uuidv7();
}

interface PmAttrs {
  blockId?: string;
  [key: string]: unknown;
}

interface PmNode {
  type: string;
  attrs?: PmAttrs;
  content?: PmNode[];
  [key: string]: unknown;
}

interface PmDoc {
  type: "doc";
  content?: PmNode[];
  [key: string]: unknown;
}

function dedupeBlockIds(doc: PmDoc): PmDoc {
  const seen = new Set<string>();
  const content = doc.content?.map((node) => {
    const id = node.attrs?.blockId;
    if (id && seen.has(id)) {
      return {
        ...node,
        attrs: { ...node.attrs, blockId: newBlockId() },
      };
    }
    if (id) seen.add(id);
    return node;
  });
  return { ...doc, content };
}

export const BlockId = Extension.create({
  name: "blockId",
  addGlobalAttributes() {
    return [
      {
        types: [
          "paragraph",
          "heading",
          "bulletList",
          "orderedList",
          "codeBlock",
        ],
        attributes: {
          blockId: {
            default: null,
            parseHTML: (el) => el.getAttribute("data-block-id"),
            renderHTML: (attrs) =>
              attrs.blockId ? { "data-block-id": attrs.blockId } : {},
          },
        },
      },
    ];
  },
  onCreate() {
    const editor = this.editor;
    const doc = dedupeBlockIds(editor.state.doc.toJSON());
    editor.commands.setContent(doc, false);
  },
  onUpdate() {
    const editor = this.editor;
    const doc = dedupeBlockIds(editor.state.doc.toJSON());
    const current = editor.state.doc.toJSON();
    if (JSON.stringify(doc) !== JSON.stringify(current)) {
      editor.commands.setContent(doc, false);
    }
  },
});
