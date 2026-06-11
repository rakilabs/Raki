import { Send } from "lucide-solid";
import { createSignal } from "solid-js";
import { Button, Textarea } from "~/shared/ui";

interface ChatInputProps {
  onSubmit: (question: string) => void;
  disabled?: boolean;
}

export function ChatInput(props: ChatInputProps) {
  const [value, setValue] = createSignal("");

  const handleSubmit = () => {
    const q = value().trim();
    if (q && !props.disabled) {
      props.onSubmit(q);
      setValue("");
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      handleSubmit();
    }
  };

  return (
    <div class="flex gap-2">
      <Textarea
        class="min-h-[60px] flex-1 resize-none"
        placeholder="Ask a question... (Cmd+Enter to send)"
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onKeyDown={handleKeyDown}
        disabled={props.disabled}
      />
      <Button
        class="self-end"
        onClick={handleSubmit}
        disabled={!value().trim() || props.disabled}
        loading={props.disabled}
      >
        <Send class="h-4 w-4" />
        <span class="hidden sm:inline">Send</span>
      </Button>
    </div>
  );
}
