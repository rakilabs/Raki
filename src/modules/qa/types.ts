import type { AnswerOutcome } from "~/shared/ipc";

export interface ChatItem {
  id: string;
  question: string;
  outcome?: AnswerOutcome;
  error?: string;
  streamingText?: string;
}
