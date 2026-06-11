import type { JSX } from "solid-js";
import {
  type Component,
  type ComponentProps,
  createUniqueId,
  Show,
  splitProps,
} from "solid-js";
import { cn } from "~/shared/lib/cn";

export interface InputProps extends ComponentProps<"input"> {
  label?: string;
  error?: string;
  helper?: string;
  leftIcon?: JSX.Element;
  rightIcon?: JSX.Element;
}

export const Input: Component<InputProps> = (props) => {
  const [local, rest] = splitProps(props, [
    "class",
    "label",
    "error",
    "helper",
    "leftIcon",
    "rightIcon",
    "id",
  ]);

  const inputId = local.id || createUniqueId();
  const errorId = `${inputId}-error`;
  const helperId = `${inputId}-helper`;
  const ariaDescribedBy = () => {
    const ids: string[] = [];
    if (local.error) ids.push(errorId);
    if (local.helper) ids.push(helperId);
    return ids.length > 0 ? ids.join(" ") : undefined;
  };

  return (
    <div class={cn("flex flex-col gap-1.5", local.class)}>
      <Show when={local.label}>
        <label for={inputId} class="text-sm font-medium text-foreground">
          {local.label}
        </label>
      </Show>
      <div class="relative flex items-center">
        <Show when={local.leftIcon}>
          <div class="pointer-events-none absolute left-3 text-muted-foreground">
            {local.leftIcon}
          </div>
        </Show>
        <input
          id={inputId}
          class={cn(
            "flex h-10 w-full rounded-md border border-border bg-background px-3 py-2 text-sm text-foreground shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
            local.leftIcon && "pl-10",
            local.rightIcon && "pr-10",
            local.error && "border-error-500 focus-visible:ring-error-300"
          )}
          aria-invalid={local.error ? "true" : undefined}
          aria-describedby={ariaDescribedBy()}
          {...rest}
        />
        <Show when={local.rightIcon}>
          <div class="pointer-events-none absolute right-3 text-muted-foreground">
            {local.rightIcon}
          </div>
        </Show>
      </div>
      <Show when={local.error}>
        <p
          id={errorId}
          class="text-xs font-medium text-error-600 dark:text-error-400"
        >
          {local.error}
        </p>
      </Show>
      <Show when={local.helper && !local.error}>
        <p id={helperId} class="text-xs text-muted-foreground">
          {local.helper}
        </p>
      </Show>
    </div>
  );
};
