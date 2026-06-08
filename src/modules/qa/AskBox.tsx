import { createSignal, Show, For } from "solid-js";
import { qaApi, type AnswerOutcome } from "./api";

const PROVIDER = "kimi";

function errMessage(e: unknown): string {
  return typeof e === "object" && e && "message" in e ? String((e as { message: unknown }).message) : String(e);
}

export function AskBox() {
  const [question, setQuestion] = createSignal("");
  const [outcome, setOutcome] = createSignal<AnswerOutcome | null>(null);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function run(fn: () => Promise<AnswerOutcome>) {
    setPending(true);
    setError(null);
    try {
      setOutcome(await fn());
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setPending(false);
    }
  }

  const ask = () => {
    const q = question().trim();
    if (q) run(() => qaApi.ask(q));
  };

  const confirmSend = () =>
    run(async () => {
      await qaApi.grant(PROVIDER); // grant consent + flip to CloudAllowed, then re-ask
      return qaApi.ask(question().trim());
    });

  return (
    <section aria-label="Ask AI (experimental)">
      <h2>Ask your notes (experimental)</h2>
      <form onSubmit={(e) => { e.preventDefault(); ask(); }}>
        <input
          placeholder="Ask a question about your notes…"
          value={question()}
          onInput={(e) => setQuestion(e.currentTarget.value)}
        />
        <button type="submit" disabled={pending()}>Ask</button>
      </form>

      <Show when={error()}>{(msg) => <p role="alert">Error: {msg()}</p>}</Show>

      <Show when={outcome()}>
        {(o) => (
          <Show
            when={o().kind === "needs_consent" ? (o() as Extract<AnswerOutcome, { kind: "needs_consent" }>) : null}
            fallback={
              <div>
                <p>{(o() as Extract<AnswerOutcome, { kind: "answer" }>).text}</p>
                <Show when={(o() as Extract<AnswerOutcome, { kind: "answer" }>).cited.length > 0}>
                  <p>Sources:</p>
                  <ul>
                    <For each={(o() as Extract<AnswerOutcome, { kind: "answer" }>).cited}>
                      {(c) => <li>{c.title}</li>}
                    </For>
                  </ul>
                </Show>
              </div>
            }
          >
            {(nc) => (
              <div>
                <p>This will send to the cloud: <strong>{nc().preview.summary}</strong></p>
                <ul>
                  <For each={nc().preview.source_titles}>{(t) => <li>{t}</li>}</For>
                </ul>
                <button type="button" disabled={pending()} onClick={confirmSend}>Send to cloud</button>
                <button type="button" onClick={() => setOutcome(null)}>Stay local</button>
              </div>
            )}
          </Show>
        )}
      </Show>
    </section>
  );
}
