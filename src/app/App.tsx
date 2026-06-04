import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";
import { NotesView } from "~/modules/notes/NotesView";

const queryClient = new QueryClient();

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <main class="container">
        <NotesView />
      </main>
    </QueryClientProvider>
  );
}
