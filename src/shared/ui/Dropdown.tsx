import { Menu } from "@ark-ui/solid";
import type { JSX } from "solid-js";
import { cn } from "~/shared/lib/cn";

export const Dropdown = Menu.Root;
export const DropdownTrigger = Menu.Trigger;

export const DropdownContent: import("solid-js").ParentComponent<{
  class?: string;
}> = (props) => (
  <Menu.Positioner>
    <Menu.Content
      class={cn(
        "z-50 min-w-[8rem] overflow-hidden rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-dropdown data-[state=open]:animate-fade-in data-[state=closed]:animate-fade-out",
        props.class
      )}
    >
      {props.children}
    </Menu.Content>
  </Menu.Positioner>
);

export interface DropdownItemProps {
  value: string;
  class?: string;
  disabled?: boolean;
  children?: JSX.Element;
}

export const DropdownItem: import("solid-js").ParentComponent<
  DropdownItemProps
> = (props) => (
  <Menu.Item
    value={props.value}
    disabled={props.disabled}
    class={cn(
      "relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none transition-colors focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
      props.class
    )}
  >
    {props.children}
  </Menu.Item>
);

export const DropdownSeparator: import("solid-js").ParentComponent<{
  class?: string;
}> = (props) => (
  <Menu.Separator class={cn("-mx-1 my-1 h-px bg-muted", props.class)} />
);

export const DropdownLabel: import("solid-js").ParentComponent<{
  class?: string;
}> = (props) => (
  <span
    class={cn(
      "px-2 py-1.5 text-xs font-semibold text-muted-foreground",
      props.class
    )}
  >
    {props.children}
  </span>
);
