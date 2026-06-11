import { Tabs as ArkTabs } from "@ark-ui/solid";
import { type ParentComponent, splitProps } from "solid-js";
import { cn } from "~/shared/lib/cn";

export const Tabs = ArkTabs.Root;

export const TabsList: ParentComponent<{ class?: string }> = (props) => (
  <ArkTabs.List
    class={cn(
      "inline-flex h-10 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground",
      props.class
    )}
  >
    {props.children}
  </ArkTabs.List>
);

export const TabsTrigger: ParentComponent<{ value: string; class?: string }> = (
  props
) => {
  const [local, rest] = splitProps(props, ["value", "class", "children"]);
  return (
    <ArkTabs.Trigger
      value={local.value}
      class={cn(
        "inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 data-[selected]:bg-background data-[selected]:text-foreground data-[selected]:shadow-sm",
        local.class
      )}
      {...rest}
    >
      {local.children}
    </ArkTabs.Trigger>
  );
};

export const TabsContent: ParentComponent<{ value: string; class?: string }> = (
  props
) => {
  const [local, rest] = splitProps(props, ["value", "class", "children"]);
  return (
    <ArkTabs.Content
      value={local.value}
      class={cn(
        "mt-2 ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
        local.class
      )}
      {...rest}
    >
      {local.children}
    </ArkTabs.Content>
  );
};
