import { createSignal, For, Show } from "solid-js";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { settingsApi, settingsKeys } from "./api";

export function SettingsPanel(props: { onClose: () => void }) {
  const queryClient = useQueryClient();
  const [activeTab, setActiveTab] = createSignal<"privacy" | "audit">("privacy");

  const settings = createQuery(() => ({
    queryKey: settingsKeys.egress,
    queryFn: () => settingsApi.getEgressSettings(),
  }));

  const setMode = createMutation(() => ({
    mutationFn: (mode: string) => settingsApi.setEgressMode(mode),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: settingsKeys.egress }),
  }));

  const grant = createMutation(() => ({
    mutationFn: (provider: string) => settingsApi.grantConsent(provider),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: settingsKeys.egress }),
  }));

  const revoke = createMutation(() => ({
    mutationFn: (provider: string) => settingsApi.revokeConsent(provider),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: settingsKeys.egress }),
  }));

  const log = createQuery(() => ({
    queryKey: settingsKeys.log,
    queryFn: () => settingsApi.listEgressLog(50),
  }));

  return (
    <div class="settings-overlay" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="settings-panel">
        <header>
          <h2>Settings</h2>
          <button onClick={props.onClose}>Close</button>
        </header>

        <nav>
          <button onClick={() => setActiveTab("privacy")} class={activeTab() === "privacy" ? "active" : ""}>
            Privacy
          </button>
          <button onClick={() => setActiveTab("audit")} class={activeTab() === "audit" ? "active" : ""}>
            Audit Log
          </button>
        </nav>

        <Show when={activeTab() === "privacy"}>
          <section>
            <h3>Egress Mode</h3>
            <label>
              <input
                type="radio"
                name="egress-mode"
                value="local_only"
                checked={settings.data?.mode === "local_only"}
                onChange={() => setMode.mutate("local_only")}
              />
              Local only — cloud calls disabled
            </label>
            <label>
              <input
                type="radio"
                name="egress-mode"
                value="cloud_allowed"
                checked={settings.data?.mode === "cloud_allowed"}
                onChange={() => setMode.mutate("cloud_allowed")}
              />
              Cloud allowed — with per-provider consent
            </label>

            <h3>Provider Consent</h3>
            <Show when={settings.data}>
              {(s) => (
                <>
                  <div>
                    <strong>kimi</strong>{" "}
                    {s().consented_providers.includes("kimi") ? (
                      <button onClick={() => revoke.mutate("kimi")} disabled={revoke.isPending}>
                        Revoke
                      </button>
                    ) : (
                      <button onClick={() => grant.mutate("kimi")} disabled={grant.isPending}>
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
                          <td>{e.token_count}</td>
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
