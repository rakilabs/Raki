import { commands, type AnswerOutcome } from "~/shared/ipc";

export type { AnswerOutcome };

export const qaApi = {
  ask: (query: string) => commands.answerQuestion(query),
  grant: (provider: string) => commands.grantCloudConsent(provider),
  revoke: (provider: string) => commands.revokeCloudConsent(provider),
};
