import {
  createMutation,
  createQuery,
  useQueryClient,
} from "@tanstack/solid-query";
import { createSignal, For, Show } from "solid-js";
import { settingsApi, settingsKeys } from "./api";

export function SettingsPanel(props: { onClose: () => void }) {
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = createSignal<"privacy" | "audit">(
    "privacy"
  );

  const settings = createQuery(() => ({
    queryKey: settingsKeys.egress,
    queryFn: () => settingsApi.getEgressSettings(),
  }));

  const grant = createMutation(() => ({
    mutationFn: (provider: string) => settingsApi.grantConsent(provider),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: settingsKeys.egress }),
  }));

  const revoke = createMutation(() => ({
    mutationFn: (provider: string) => settingsApi.revokeConsent(provider),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: settingsKeys.egress }),
  }));

  const log = createQuery(() => ({
    queryKey: settingsKeys.log,
    queryFn: () => settingsApi.listEgressLog(50),
  }));

  return (
    <div
      class="settings-overlay"
      onClick={(e) => e.target === e.currentTarget && props.onClose()}
    >
      <div class="settings-panel">
        <header>
          <h2>Settings</h2>
          <button onClick={props.onClose}>Close</button>
        </header>

        <nav>
          <button
            onClick={() => setActiveTab("privacy")}
            class={activeTab() === "privacy" ? "active" : ""}
          >
            Privacy
          </button>
          <button
            onClick={() => setActiveTab("audit")}
            class={activeTab() === "audit" ? "active" : ""}
          >
            Audit Log
          </button>
        </nav>

        <Show when={activeTab() === "privacy"}>
          <section>
            <p>
              Local providers (e.g., Ollama) run on your device and need no
              consent. Cloud providers require your explicit approval before
              each can be used.
            </p>

            <h3>Cloud Provider Consent</h3>
            <Show when={settings.data}>
              {(s) => (
                <>
                  <div>
                    <strong>kimi</strong>{" "}
                    {s().consented_providers.includes("kimi") ? (
                      <button
                        onClick={() => revoke.mutate("kimi")}
                        disabled={revoke.isPending}
                      >
                        Revoke
                      </button>
                    ) : (
                      <button
                        onClick={() => grant.mutate("kimi")}
                        disabled={grant.isPending}
                      >
                        Grant
                      </button>
                    )}
                  </div>
                </>
              )}
            </Show>
          </section>
        </Show>

        <Show when={activeTab() === "audit"}>
          <section>
            <h3>Recent Egress Events</h3>
            <Show when={log.data}>
              {(entries) => (
                <table>
                  <thead>
                    <tr>
                      <th>Time</th>
                      <th>Provider</th>
                      <th>Model</th>
                      <th>Tokens</th>
                      <th>Sources</th>
                      <th>Success</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={entries()}>
                      {(e) => (
                        <tr>
                          <td>{new Date(e.created_at).toLocaleString()}</td>
                          <td>{e.provider}</td>
                          <td>{e.model}</td>
                          <td>{Number(e.token_count)}</td>
                          <td>{e.source_count}</td>
                          <td>{e.success ? "✓" : "✗"}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              )}
            </Show>
            <Show when={!log.data || log.data.length === 0}>
              <p>No egress events recorded.</p>
            </Show>
          </section>
        </Show>
      </div>
    </div>
  );
}
