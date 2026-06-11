import { createEffect, createSignal } from "solid-js";

export function useDebounce<T>(value: () => T, delay = 300): () => T {
  const [debounced, setDebounced] = createSignal(value());

  createEffect(() => {
    const v = value();
    const timer = setTimeout(() => setDebounced(() => v), delay);
    return () => clearTimeout(timer);
  });

  return debounced;
}
