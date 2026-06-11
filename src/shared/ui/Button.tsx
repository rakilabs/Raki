import { type Component, type ComponentProps, splitProps } from "solid-js";
import { cn } from "~/shared/lib/cn";
import { cva, type VariantProps } from "~/shared/lib/cva";
import { Spinner } from "./Spinner";

const buttonVariants = cva(
  "btn-base inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        primary:
          "bg-primary-600 text-white hover:bg-primary-700 active:bg-primary-800 dark:bg-primary-600 dark:hover:bg-primary-500",
        secondary:
          "bg-secondary-100 text-secondary-800 hover:bg-secondary-200 active:bg-secondary-300 dark:bg-secondary-900 dark:text-secondary-200 dark:hover:bg-secondary-800",
        ghost:
          "bg-transparent text-foreground hover:bg-muted active:bg-muted/80",
        outline:
          "border border-border bg-transparent text-foreground hover:bg-muted active:bg-muted/80",
        destructive:
          "bg-error-600 text-white hover:bg-error-700 active:bg-error-800 dark:bg-error-600 dark:hover:bg-error-500",
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-10 px-4 py-2",
        lg: "h-12 px-6 text-base",
        icon: "h-10 w-10 p-2",
      },
    },
    defaultVariants: {
      variant: "primary",
      size: "md",
    },
  }
);

export interface ButtonProps
  extends ComponentProps<"button">,
    VariantProps<typeof buttonVariants> {
  loading?: boolean;
}

export const Button: Component<ButtonProps> = (props) => {
  const [local, rest] = splitProps(props, [
    "class",
    "variant",
    "size",
    "loading",
    "children",
    "disabled",
  ]);

  return (
    <button
      class={cn(
        buttonVariants({ variant: local.variant, size: local.size }),
        local.class
      )}
      disabled={local.disabled || local.loading}
      {...rest}
    >
      {local.loading && <Spinner size="sm" class="text-current" />}
      {local.children}
    </button>
  );
};
