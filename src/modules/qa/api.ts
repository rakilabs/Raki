import { commands, type AnswerOutcome } from "~/shared/ipc";

export type { AnswerOutcome };

export const qaApi = {
  ask: (query: string) => commands.answerQuestion(query),
  grant: (provider: string) => commands.grantProviderConsent(provider),
  revoke: (provider: string) => commands.revokeProviderConsent(provider),
};
