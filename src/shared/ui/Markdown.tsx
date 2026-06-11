import DOMPurify from "dompurify";
import { marked } from "marked";
import { type Component, createMemo } from "solid-js";

export interface MarkdownProps {
  content: string;
  class?: string;
}

export const Markdown: Component<MarkdownProps> = (props) => {
  const html = createMemo(() => {
    const raw = marked.parse(props.content, { async: false }) as string;
    return DOMPurify.sanitize(raw);
  });

  return <div class={props.class} innerHTML={html()} />;
};
