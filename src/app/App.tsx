import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";
import { ErrorBoundary } from "solid-js";
import { ThemeProvider } from "~/app/providers/ThemeProvider";
import { Router } from "~/app/Router";
import { ToastProvider } from "~/shared/ui";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5,
      retry: 1,
    },
  },
});

export function App() {
  return (
    <ErrorBoundary
      fallback={(err) => (
        <div
          style={{
            "background-color": "#fee",
            color: "#900",
            padding: "20px",
            "font-family": "monospace",
          }}
        >
          <h1>App Error</h1>
          <pre>{err instanceof Error ? err.message : String(err)}</pre>
          <pre>{err instanceof Error ? err.stack : ""}</pre>
        </div>
      )}
    >
      <QueryClientProvider client={queryClient}>
        <ThemeProvider defaultTheme="system">
          <ToastProvider>
            <ErrorBoundary
              fallback={(err) => (
                <div
                  style={{
                    "background-color": "#ffe",
                    color: "#660",
                    padding: "20px",
                  }}
                >
                  <h2>Router Error</h2>
                  <pre>{err instanceof Error ? err.message : String(err)}</pre>
                </div>
              )}
            >
              <Router />
            </ErrorBoundary>
          </ToastProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </ErrorBoundary>
  );
}
