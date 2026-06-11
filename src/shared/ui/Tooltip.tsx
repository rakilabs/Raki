import { Tooltip as ArkTooltip } from "@ark-ui/solid";
import type { ParentComponent } from "solid-js";
import { cn } from "~/shared/lib/cn";

export const Tooltip: ParentComponent = (props) => (
  <ArkTooltip.Root openDelay={300} closeDelay={100}>
    {props.children}
  </ArkTooltip.Root>
);

export const TooltipTrigger = ArkTooltip.Trigger;

export const TooltipContent: ParentComponent<{ class?: string }> = (props) => (
  <ArkTooltip.Positioner>
    <ArkTooltip.Content
      class={cn(
        "z-50 overflow-hidden rounded-md border border-border bg-popover px-3 py-1.5 text-sm text-popover-foreground shadow-dropdown animate-fade-in",
        props.class
      )}
    >
      {props.children}
    </ArkTooltip.Content>
  </ArkTooltip.Positioner>
);
