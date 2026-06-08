import { createSignal, Show } from "solid-js";
import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";
import { NotesView } from "~/modules/notes/NotesView";
import { AskBox } from "~/modules/qa/AskBox";

const queryClient = new QueryClient();

export function App() {
  const [qaEnabled, setQaEnabled] = createSignal(localStorage.getItem("raki.qa.enabled") === "1");
  function toggleQa(on: boolean) {
    setQaEnabled(on);
    localStorage.setItem("raki.qa.enabled", on ? "1" : "0");
  }

  return (
    <QueryClientProvider client={queryClient}>
      <main class="container">
        <label>
          <input type="checkbox" checked={qaEnabled()} onChange={(e) => toggleQa(e.currentTarget.checked)} />
          Enable experimental retrieval diagnostics
        </label>
        <Show when={qaEnabled()}>
          <AskBox />
        </Show>
        <NotesView />
      </main>
    </QueryClientProvider>
  );
}
