import { For, Show } from "solid-js";
import type { EgressPreviewDto } from "~/shared/ipc";
import {
  Button,
  Modal,
  ModalContent,
  ModalDescription,
  ModalFooter,
  ModalHeader,
  ModalTitle,
} from "~/shared/ui";

interface ConsentDialogProps {
  open: boolean;
  onClose: () => void;
  preview?: EgressPreviewDto;
  onConfirm: () => void;
}

export function ConsentDialog(props: ConsentDialogProps) {
  return (
    <Modal
      open={props.open}
      onOpenChange={(details) => !details.open && props.onClose()}
    >
      <ModalContent>
        <ModalHeader>
          <ModalTitle>Cloud Provider Consent</ModalTitle>
          <ModalDescription>
            This query will send data to a cloud provider. Review what will
            leave your device.
          </ModalDescription>
        </ModalHeader>

        <Show when={props.preview}>
          {(p) => (
            <div class="space-y-3 py-4">
              <div class="rounded-md bg-muted p-3">
                <p class="text-sm font-medium text-foreground">{p().summary}</p>
              </div>
              <Show when={p().source_titles.length > 0}>
                <div>
                  <p class="mb-1 text-xs font-medium text-muted-foreground">
                    Sources that will be sent:
                  </p>
                  <ul class="space-y-1">
                    <For each={p().source_titles}>
                      {(title) => (
                        <li class="flex items-center gap-2 text-sm text-foreground">
                          <span class="h-1.5 w-1.5 rounded-full bg-primary-500" />
                          {title}
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
              </Show>
            </div>
          )}
        </Show>

        <ModalFooter>
          <Button variant="outline" onClick={props.onClose}>
            Stay Local
          </Button>
          <Button onClick={props.onConfirm}>Send to Cloud</Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
