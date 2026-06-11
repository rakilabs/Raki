import { type Component, createSignal, Show, splitProps } from "solid-js";
import { cn } from "~/shared/lib/cn";

export interface AvatarProps {
  src?: string;
  alt?: string;
  initials?: string;
  size?: "sm" | "md" | "lg";
  class?: string;
}

export const Avatar: Component<AvatarProps> = (props) => {
  const [local, rest] = splitProps(props, [
    "src",
    "alt",
    "initials",
    "size",
    "class",
  ]);
  const [error, setError] = createSignal(false);

  const sizeClasses = {
    sm: "h-8 w-8 text-xs",
    md: "h-10 w-10 text-sm",
    lg: "h-14 w-14 text-base",
  };

  const showFallback = () => !local.src || error();

  return (
    <div
      class={cn(
        "relative inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted font-medium text-muted-foreground",
        sizeClasses[local.size || "md"],
        local.class
      )}
      {...rest}
    >
      <Show when={!showFallback()}>
        <img
          src={local.src}
          alt={local.alt || ""}
          class="h-full w-full object-cover"
          onError={() => setError(true)}
        />
      </Show>
      <Show when={showFallback()}>
        <span class="select-none">{local.initials || "?"}</span>
      </Show>
    </div>
  );
};
