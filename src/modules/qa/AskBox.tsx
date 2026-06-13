import { createSignal, For, Show } from "solid-js";
import { type AnswerOutcome, qaApi } from "./api";

const PROVIDER = "kimi";

function errMessage(e: unknown): string {
  return typeof e === "object" && e && "message" in e
    ? String((e as { message: unknown }).message)
    : String(e);
}

function isNeedsConsent(
  o: AnswerOutcome
): o is Extract<AnswerOutcome, { kind: "needs_consent" }> {
  return o.kind === "needs_consent";
}

export function AskBox() {
  const [question, setQuestion] = createSignal("");
  const [outcome, setOutcome] = createSignal<AnswerOutcome | null>(null);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [lastReqId, setLastReqId] = createSignal(0);

  async function run(fn: () => Promise<AnswerOutcome>) {
    const reqId = lastReqId() + 1;
    setLastReqId(reqId);
    setPending(true);
    setError(null);
    try {
      const result = await fn();
      if (lastReqId() === reqId) {
        setOutcome(result);
      }
    } catch (e) {
      if (lastReqId() === reqId) {
        setError(errMessage(e));
      }
    } finally {
      if (lastReqId() === reqId) {
        setPending(false);
      }
    }
  }

  const ask = () => {
    const q = question().trim();
    if (q) run(() => qaApi.ask(q));
  };

  const confirmSend = () =>
    run(async () => {
      await qaApi.grant(PROVIDER); // grant provider consent, then re-ask
      return qaApi.ask(question().trim());
    });

  function renderOutcome(o: AnswerOutcome) {
    if (isNeedsConsent(o)) {
      return (
        <div>
          <p>
            This will send to the cloud: <strong>{o.preview.summary}</strong>
          </p>
          <ul>
            <For each={o.preview.source_titles}>{(t) => <li>{t}</li>}</For>
          </ul>
          <button type="button" disabled={pending()} onClick={confirmSend}>
            Send to cloud
          </button>
          <button type="button" onClick={() => setOutcome(null)}>
            Stay local
          </button>
        </div>
      );
    }
    return (
      <div>
        <p>{o.text}</p>
        <Show when={o.cited.length > 0}>
          <p>Sources:</p>
          <ul>
            <For each={o.cited}>{(c) => <li>{c.title}</li>}</For>
          </ul>
        </Show>
      </div>
    );
  }

  return (
    <section aria-label="Ask AI (experimental)">
      <h2>Ask your notes (experimental)</h2>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          ask();
        }}
      >
        <input
          placeholder="Ask a question about your notes…"
          value={question()}
          onInput={(e) => setQuestion(e.currentTarget.value)}
        />
        <button type="submit" disabled={pending()}>
          Ask
        </button>
      </form>

      <Show when={error()}>{(msg) => <p role="alert">Error: {msg()}</p>}</Show>

      <Show when={outcome()} keyed>
        {(o) => renderOutcome(o)}
      </Show>
    </section>
  );
}
