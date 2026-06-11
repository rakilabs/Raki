import { createQuery } from "@tanstack/solid-query";
import { CheckCircle, ClipboardList, XCircle } from "lucide-solid";
import { For, Show } from "solid-js";
import { settingsApi, settingsKeys } from "~/modules/settings/api";
import {
  Badge,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Skeleton,
} from "~/shared/ui";

export function AuditTab() {
  const log = createQuery(() => ({
    queryKey: settingsKeys.log,
    queryFn: () => settingsApi.listEgressLog(50),
  }));

  return (
    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2">
          <ClipboardList class="h-5 w-5 text-primary-600" />
          Recent Egress Events
        </CardTitle>
        <CardDescription>
          History of data sent to cloud providers
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Show when={log.isLoading}>
          <div class="space-y-2">
            <Skeleton height="40px" />
            <Skeleton height="40px" />
            <Skeleton height="40px" />
          </div>
        </Show>

        <Show when={!log.isLoading && (!log.data || log.data.length === 0)}>
          <div class="flex flex-col items-center justify-center py-12 text-muted-foreground">
            <ClipboardList class="mb-2 h-8 w-8 opacity-50" />
            <p class="text-sm">No egress events recorded</p>
          </div>
        </Show>

        <Show when={log.data && log.data.length > 0}>
          <div class="overflow-x-auto">
            <table class="w-full text-sm">
              <thead>
                <tr class="border-b border-border text-left text-muted-foreground">
                  <th class="pb-2 pr-4 font-medium">Time</th>
                  <th class="pb-2 pr-4 font-medium">Provider</th>
                  <th class="pb-2 pr-4 font-medium">Model</th>
                  <th class="pb-2 pr-4 font-medium">Tokens</th>
                  <th class="pb-2 pr-4 font-medium">Sources</th>
                  <th class="pb-2 font-medium">Status</th>
                </tr>
              </thead>
              <tbody>
                <For each={log.data ?? []}>
                  {(entry) => (
                    <tr class="border-b border-border last:border-0">
                      <td class="py-3 pr-4 whitespace-nowrap">
                        {new Date(entry.created_at).toLocaleString()}
                      </td>
                      <td class="py-3 pr-4">{entry.provider}</td>
                      <td class="py-3 pr-4">{entry.model}</td>
                      <td class="py-3 pr-4">{Number(entry.token_count)}</td>
                      <td class="py-3 pr-4">{entry.source_count}</td>
                      <td class="py-3">
                        {entry.success ? (
                          <Badge variant="success" class="gap-1">
                            <CheckCircle class="h-3 w-3" />
                            Success
                          </Badge>
                        ) : (
                          <Badge variant="destructive" class="gap-1">
                            <XCircle class="h-3 w-3" />
                            Failed
                          </Badge>
                        )}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </CardContent>
    </Card>
  );
}
