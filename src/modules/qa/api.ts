import { type AnswerOutcome, commands } from "~/shared/ipc";

export type { AnswerOutcome };

export const qaApi = {
  ask: (query: string) => commands.answerQuestion(query),
  grant: (provider: string) => commands.grantProviderConsent(provider),
  revoke: (provider: string) => commands.revokeProviderConsent(provider),
};

/**
 * Streaming API infrastructure for SSE/NDJSON consumption.
 * Provides abortable requests with retry logic.
 */
export const streamApi = {
  /**
   * Read an NDJSON stream line by line.
   */
  async *readStream(response: Response): AsyncIterable<string> {
    const reader = response.body?.getReader();
    if (!reader) return;

    const decoder = new TextDecoder();
    let buffer = "";

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() || "";

        for (const line of lines) {
          const trimmed = line.trim();
          if (trimmed) yield trimmed;
        }
      }

      if (buffer.trim()) yield buffer.trim();
    } finally {
      reader.releaseLock();
    }
  },

  /**
   * Fetch with automatic retry and exponential backoff.
   */
  async fetchWithRetry(
    url: string,
    options: RequestInit & { retries?: number; backoff?: number } = {}
  ): Promise<Response> {
    const { retries = 3, backoff = 300, ...init } = options;
    let lastError: Error | undefined;

    for (let i = 0; i <= retries; i++) {
      try {
        const response = await fetch(url, init);
        if (response.ok) return response;
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      } catch (e) {
        lastError = e instanceof Error ? e : new Error(String(e));
        if (i < retries) {
          await new Promise((r) => setTimeout(r, backoff * 2 ** i));
        }
      }
    }

    throw lastError;
  },

  /**
   * Create an abortable stream request.
   * Returns the controller so the caller can abort.
   */
  createAbortableStream(
    url: string,
    options: RequestInit = {}
  ): {
    controller: AbortController;
    promise: Promise<Response>;
  } {
    const controller = new AbortController();
    const promise = fetch(url, { ...options, signal: controller.signal });
    return { controller, promise };
  },
};
