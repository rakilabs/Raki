import { commands } from "~/shared/ipc";

export const settingsKeys = {
  all: ["settings"] as const,
  egress: ["settings", "egress"] as const,
  log: ["settings", "log"] as const,
};

export const settingsApi = {
  getEgressSettings: () => commands.getEgressSettings(),
  grantConsent: (provider: string) => commands.grantProviderConsent(provider),
  revokeConsent: (provider: string) => commands.revokeProviderConsent(provider),
  listEgressLog: (limit: number) => commands.listEgressLog(limit),
};
