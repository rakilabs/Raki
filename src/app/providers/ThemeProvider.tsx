import {
  createContext,
  createSignal,
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

export const ThemeProvider: ParentComponent<{ defaultTheme?: Theme }> = (
  props
) => {
  const [theme, setThemeState] = createSignal<Theme>(
    (localStorage.getItem(STORAGE_KEY) as Theme) ||
      props.defaultTheme ||
      "system"
  );

  const resolvedTheme = (): ResolvedTheme => {
    const t = theme();
    return t === "system" ? getSystemTheme() : t;
  };

  const applyTheme = (t: ResolvedTheme) => {
    const root = document.documentElement;
    root.classList.remove("light", "dark");
    root.classList.add(t);
  };

  // Apply on mount and when theme changes
  applyTheme(resolvedTheme());

  const setTheme = (t: Theme) => {
    localStorage.setItem(STORAGE_KEY, t);
    setThemeState(t);
    applyTheme(t === "system" ? getSystemTheme() : t);
  };

  // Listen for system theme changes
  const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  mediaQuery.addEventListener("change", () => {
    if (theme() === "system") {
      applyTheme(getSystemTheme());
    }
  });

  const toggle = () => {
    setTheme(resolvedTheme() === "dark" ? "light" : "dark");
  };

  return (
    <ThemeContext.Provider value={{ theme, resolvedTheme, setTheme, toggle }}>
      {props.children}
    </ThemeContext.Provider>
  );
};

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within a ThemeProvider");
  return ctx;
}
