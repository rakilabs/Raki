import {
  createContext,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  type ParentComponent,
  useContext,
} from "solid-js";

type Theme = "light" | "dark" | "system";
type ResolvedTheme = "light" | "dark";

interface ThemeContextValue {
  theme: () => Theme;
  resolvedTheme: () => ResolvedTheme;
  setTheme: (theme: Theme) => void;
  toggle: () => void;
}

const ThemeContext = createContext<ThemeContextValue>();

const STORAGE_KEY = "raki.theme";

function getSystemTheme(): ResolvedTheme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

function applyThemeClass(t: ResolvedTheme) {
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  root.classList.add(t);
  root.setAttribute("data-theme", t);
}

export const ThemeProvider: ParentComponent<{ defaultTheme?: Theme }> = (
  props
) => {
  const stored = localStorage.getItem(STORAGE_KEY) as Theme | null;
  const validStored: Theme | null =
    stored && ["light", "dark", "system"].includes(stored) ? stored : null;

  const [theme, setThemeState] = createSignal<Theme>(
    validStored || props.defaultTheme || "system"
  );

  const resolvedTheme = createMemo<ResolvedTheme>(() => {
    const t = theme();
    return t === "system" ? getSystemTheme() : t;
  });

  // Sync class to DOM whenever resolved theme changes
  createEffect(() => {
    applyThemeClass(resolvedTheme());
  });

  const setTheme = (t: Theme) => {
    localStorage.setItem(STORAGE_KEY, t);
    setThemeState(t);
  };

  const toggle = () => {
    const current = resolvedTheme();
    setTheme(current === "dark" ? "light" : "dark");
  };

  // Listen for system theme changes
  onMount(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => {
      if (theme() === "system") {
        applyThemeClass(getSystemTheme());
      }
    };
    mediaQuery.addEventListener("change", handler);
    onCleanup(() => mediaQuery.removeEventListener("change", handler));
  });

  return (
    <ThemeContext.Provider
      value={{ theme, resolvedTheme: resolvedTheme, setTheme, toggle }}
    >
      {props.children}
    </ThemeContext.Provider>
  );
};

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within a ThemeProvider");
  return ctx;
}
