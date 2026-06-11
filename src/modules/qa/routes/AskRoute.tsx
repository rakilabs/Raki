import { Trash2 } from "lucide-solid";
import { createSignal, For, Show } from "solid-js";
import { qaApi } from "~/modules/qa/api";
import { ChatInput } from "~/modules/qa/components/ChatInput";
import { ChatMessage } from "~/modules/qa/components/ChatMessage";
import { ConsentDialog } from "~/modules/qa/components/ConsentDialog";
import type { ChatItem } from "~/modules/qa/types";
import { Button, Card, useToast } from "~/shared/ui";

export default function AskRoute() {
  const toast = useToast();
  const [items, setItems] = createSignal<ChatItem[]>([]);
  const [pendingId, setPendingId] = createSignal<string | null>(null);
  const [consentItem, setConsentItem] = createSignal<ChatItem | null>(null);

  const handleAsk = async (question: string) => {
    const id = `q-${Date.now()}`;
    const item: ChatItem = { id, question };
    setItems((prev) => [...prev, item]);
    setPendingId(id);

    try {
      const outcome = await qaApi.ask(question);
      if (outcome.kind === "needs_consent") {
        setConsentItem({ ...item, outcome });
        setPendingId(null);
        return;
      }
      setItems((prev) =>
        prev.map((i) => (i.id === id ? { ...i, outcome } : i))
      );
    } catch (e) {
      const msg =
        typeof e === "object" && e && "message" in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setItems((prev) =>
        prev.map((i) => (i.id === id ? { ...i, error: msg } : i))
      );
      toast.add({ type: "error", message: msg });
    } finally {
      setPendingId(null);
    }
  };

  const handleConfirmConsent = async () => {
    const item = consentItem();
    if (!item) return;
    setConsentItem(null);
    setPendingId(item.id);

    try {
      await qaApi.grant("kimi");
      const outcome = await qaApi.ask(item.question);
      setItems((prev) =>
        prev.map((i) => (i.id === item.id ? { ...i, outcome } : i))
      );
    } catch (e) {
      const msg =
        typeof e === "object" && e && "message" in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setItems((prev) =>
        prev.map((i) => (i.id === item.id ? { ...i, error: msg } : i))
      );
      toast.add({ type: "error", message: msg });
    } finally {
      setPendingId(null);
    }
  };

  const clearHistory = () => setItems([]);

  return (
    <div class="flex h-full flex-col p-4">
      <div class="mb-4 flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold">Ask your notes</h1>
          <p class="text-sm text-muted-foreground">
            Ask questions about your knowledge base
          </p>
        </div>
        <Show when={items().length > 0}>
          <Button variant="ghost" size="sm" onClick={clearHistory}>
            <Trash2 class="h-4 w-4" />
            Clear
          </Button>
        </Show>
      </div>

      <Card class="flex flex-1 flex-col overflow-hidden">
        <div class="flex-1 overflow-auto p-4">
          <Show when={items().length === 0}>
            <div class="flex h-full flex-col items-center justify-center text-muted-foreground">
              <p class="text-lg font-medium">Start a conversation</p>
              <p class="text-sm">Ask anything about your notes</p>
            </div>
          </Show>

          <div class="flex flex-col gap-4">
            <For each={items()}>
              {(item) => (
                <ChatMessage item={item} isLoading={pendingId() === item.id} />
              )}
            </For>
          </div>
        </div>

        <div class="border-t border-border p-4">
          <ChatInput onSubmit={handleAsk} disabled={pendingId() !== null} />
        </div>
      </Card>

      <ConsentDialog
        open={consentItem() !== null}
        onClose={() => setConsentItem(null)}
        preview={(() => {
          const item = consentItem();
          return item?.outcome?.kind === "needs_consent"
            ? item.outcome.preview
            : undefined;
        })()}
        onConfirm={handleConfirmConsent}
      />
    </div>
  );
}
