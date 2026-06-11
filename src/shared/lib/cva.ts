import { cva as baseCva, type VariantProps } from "class-variance-authority";
import { cn } from "./cn";

export type { VariantProps };

/**
 * Wrapper around class-variance-authority that composes with `cn` automatically.
 */
export function cva<T extends Record<string, Record<string, string>>>(
  base: string,
  config?: {
    variants?: T;
    defaultVariants?: { [K in keyof T]?: keyof T[K] };
    compoundVariants?: Array<
      { [K in keyof T]?: keyof T[K] | Array<keyof T[K]> } & { class: string }
    >;
  }
) {
  const fn = baseCva(base, config as any);
  return (props?: { [K in keyof T]?: keyof T[K] } & { class?: string }) => {
    return cn(fn(props as any), props?.class);
  };
}
