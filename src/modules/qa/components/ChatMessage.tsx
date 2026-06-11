import { AlertCircle, Bot, User } from "lucide-solid";
import { For, Show } from "solid-js";
import type { ChatItem } from "~/modules/qa/types";
import { Badge, Card, Markdown, Spinner } from "~/shared/ui";

export interface ChatMessageProps {
  item: ChatItem;
  isLoading: boolean;
}

export function ChatMessage(props: ChatMessageProps) {
  const outcome = () => {
    const o = props.item.outcome;
    return o && o.kind === "answer" ? o : null;
  };

  return (
    <div class="flex gap-3">
      <div class="mt-1 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary-100 text-primary-700 dark:bg-primary-900 dark:text-primary-300">
        <User class="h-4 w-4" />
      </div>
      <div class="flex-1 space-y-3">
        <p class="text-sm font-medium text-foreground">{props.item.question}</p>

        <Show when={props.isLoading}>
          <div class="flex items-center gap-2 text-muted-foreground">
            <Spinner size="sm" />
            <span class="text-sm">Thinking...</span>
          </div>
        </Show>

        <Show when={props.item.error}>
          {(err) => (
            <div class="flex items-center gap-2 rounded-md bg-error-50 p-3 text-error-700 dark:bg-error-950 dark:text-error-300">
              <AlertCircle class="h-4 w-4 shrink-0" />
              <span class="text-sm">{err()}</span>
            </div>
          )}
        </Show>

        <Show when={outcome()}>
          {(o) => (
            <Card class="bg-muted/50">
              <div class="flex items-start gap-2 p-4">
                <Bot class="mt-0.5 h-4 w-4 shrink-0 text-primary-600 dark:text-primary-400" />
                <div class="flex-1">
                  <div class="prose text-sm text-foreground">
                    <Markdown content={o().text} />
                  </div>

                  <Show when={o().cited.length > 0}>
                    <div class="mt-3 flex flex-wrap gap-2">
                      <span class="text-xs text-muted-foreground">
                        Sources:
                      </span>
                      <For each={o().cited}>
                        {(cite) => (
                          <Badge variant="secondary" class="text-xs">
                            {cite.title}
                          </Badge>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>
              </div>
            </Card>
          )}
        </Show>
      </div>
    </div>
  );
}
