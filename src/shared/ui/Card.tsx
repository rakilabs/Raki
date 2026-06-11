import type { ParentComponent } from "solid-js";
import { cn } from "~/shared/lib/cn";

export interface CardProps {
  class?: string;
  variant?: "default" | "hoverable";
}

export const Card: ParentComponent<CardProps> = (props) => {
  const className = () =>
    cn(
      "rounded-xl border border-border bg-card text-card-foreground shadow-card",
      props.variant === "hoverable" &&
        "transition-shadow hover:shadow-dropdown cursor-pointer",
      props.class
    );

  return <div class={className()}>{props.children}</div>;
};

export const CardHeader: ParentComponent<{ class?: string }> = (props) => (
  <div class={cn("flex flex-col gap-1.5 p-6", props.class)}>
    {props.children}
  </div>
);

export const CardTitle: ParentComponent<{ class?: string }> = (props) => (
  <h3 class={cn("font-semibold leading-none tracking-tight", props.class)}>
    {props.children}
  </h3>
);

export const CardDescription: ParentComponent<{ class?: string }> = (props) => (
  <p class={cn("text-sm text-muted-foreground", props.class)}>
    {props.children}
  </p>
);

export const CardContent: ParentComponent<{ class?: string }> = (props) => (
  <div class={cn("p-6 pt-0", props.class)}>{props.children}</div>
);

export const CardFooter: ParentComponent<{ class?: string }> = (props) => (
  <div class={cn("flex items-center p-6 pt-0", props.class)}>
    {props.children}
  </div>
);
