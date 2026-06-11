import type { Component } from "solid-js";
import { cn } from "~/shared/lib/cn";

export interface SkeletonProps {
  class?: string;
  width?: string;
  height?: string;
}

export const Skeleton: Component<SkeletonProps> = (props) => (
  <div
    class={cn("animate-pulse rounded-md bg-muted", props.class)}
    style={{
      width: props.width,
      height: props.height,
    }}
    aria-hidden="true"
  />
);
