import {
  type Component,
  type ComponentProps,
  Show,
  splitProps,
} from "solid-js";
import { cn } from "~/shared/lib/cn";

export interface TextareaProps extends ComponentProps<"textarea"> {
  label?: string;
  error?: string;
  helper?: string;
}

export const Textarea: Component<TextareaProps> = (props) => {
  const [local, rest] = splitProps(props, [
    "class",
    "label",
    "error",
    "helper",
    "id",
  ]);

  const inputId = () => local.id || Math.random().toString(36).slice(2);
  const errorId = () => `${inputId()}-error`;
  const helperId = () => `${inputId()}-helper`;
  const ariaDescribedBy = () => {
    const ids: string[] = [];
    if (local.error) ids.push(errorId());
    if (local.helper) ids.push(helperId());
    return ids.length > 0 ? ids.join(" ") : undefined;
  };

  return (
    <div class={cn("flex flex-col gap-1.5", local.class)}>
      <Show when={local.label}>
        <label for={inputId()} class="text-sm font-medium text-foreground">
          {local.label}
        </label>
      </Show>
      <textarea
        id={inputId()}
        class={cn(
          "flex min-h-[80px] w-full rounded-md border border-border bg-background px-3 py-2 text-sm text-foreground shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-y",
          local.error && "border-error-500 focus-visible:ring-error-300"
        )}
        aria-invalid={local.error ? "true" : undefined}
        aria-describedby={ariaDescribedBy()}
        {...rest}
      />
      <Show when={local.error}>
        <p
          id={errorId()}
          class="text-xs font-medium text-error-600 dark:text-error-400"
        >
          {local.error}
        </p>
      </Show>
      <Show when={local.helper && !local.error}>
        <p id={helperId()} class="text-xs text-muted-foreground">
          {local.helper}
        </p>
      </Show>
    </div>
  );
};
