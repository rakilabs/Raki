import { Dialog } from "@ark-ui/solid";
import { X } from "lucide-solid";
import { type ParentComponent, splitProps } from "solid-js";
import { cn } from "~/shared/lib/cn";

export const Modal = Dialog.Root;
export const ModalTrigger = Dialog.Trigger;

export const ModalContent: ParentComponent<{ class?: string }> = (props) => {
  const [local, rest] = splitProps(props, ["class", "children"]);

  return (
    <Dialog.Positioner class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <Dialog.Backdrop class="fixed inset-0 bg-neutral-950/50 backdrop-blur-sm data-[state=open]:animate-fade-in data-[state=closed]:animate-fade-out" />
      <Dialog.Content
        class={cn(
          "relative w-full max-w-lg rounded-xl border border-border bg-card p-6 text-card-foreground shadow-modal data-[state=open]:animate-slide-up data-[state=closed]:animate-fade-out",
          local.class
        )}
        {...rest}
      >
        {local.children}
        <Dialog.CloseTrigger class="absolute right-4 top-4 rounded-sm opacity-70 ring-offset-background transition-opacity hover:opacity-100 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:pointer-events-none">
          <X class="h-4 w-4" />
          <span class="sr-only">Close</span>
        </Dialog.CloseTrigger>
      </Dialog.Content>
    </Dialog.Positioner>
  );
};

export const ModalHeader: ParentComponent<{ class?: string }> = (props) => (
  <div
    class={cn("flex flex-col gap-1.5 text-center sm:text-left", props.class)}
  >
    {props.children}
  </div>
);

export const ModalFooter: ParentComponent<{ class?: string }> = (props) => (
  <div
    class={cn(
      "mt-6 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
      props.class
    )}
  >
    {props.children}
  </div>
);

export const ModalTitle = Dialog.Title;
export const ModalDescription = Dialog.Description;
export const ModalClose = Dialog.CloseTrigger;
