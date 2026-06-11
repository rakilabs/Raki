import { type ParentComponent, splitProps } from "solid-js";
import { cva, type VariantProps } from "~/shared/lib/cva";

const badgeVariants = cva(
  "inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
  {
    variants: {
      variant: {
        default:
          "border-transparent bg-primary-100 text-primary-800 hover:bg-primary-200 dark:bg-primary-900 dark:text-primary-200",
        secondary:
          "border-transparent bg-secondary-100 text-secondary-800 hover:bg-secondary-200 dark:bg-secondary-900 dark:text-secondary-200",
        outline: "border-border text-foreground hover:bg-muted",
        destructive:
          "border-transparent bg-error-100 text-error-800 hover:bg-error-200 dark:bg-error-900 dark:text-error-200",
        success:
          "border-transparent bg-success-100 text-success-800 hover:bg-success-200 dark:bg-success-900 dark:text-success-200",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
);

export interface BadgeProps extends VariantProps<typeof badgeVariants> {
  class?: string;
}

export const Badge: ParentComponent<BadgeProps> = (props) => {
  const [local, rest] = splitProps(props, ["class", "variant", "children"]);
  return (
    <div
      class={badgeVariants({ variant: local.variant, class: local.class })}
      {...rest}
    >
      {local.children}
    </div>
  );
};
