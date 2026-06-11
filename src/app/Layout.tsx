import { A, type RouteSectionProps, useLocation } from "@solidjs/router";
import {
  FileText,
  Laptop,
  MessageCircle,
  Moon,
  Settings,
  Sun,
} from "lucide-solid";
import { Match, Switch } from "solid-js";
import { useTheme } from "~/app/providers/ThemeProvider";
import { cn } from "~/shared/lib/cn";

export default function Layout(props: RouteSectionProps) {
  const location = useLocation();
  const theme = useTheme();

  const navItems = [
    { href: "/notes", label: "Notes", icon: FileText },
    { href: "/ask", label: "Ask AI", icon: MessageCircle },
    { href: "/settings", label: "Settings", icon: Settings },
  ];

  return (
    <div class="flex h-screen bg-background text-foreground">
      {/* Sidebar */}
      <aside class="flex w-16 flex-col items-center border-r border-border bg-card py-4 md:w-56 md:items-stretch md:px-3">
        <div class="mb-6 flex items-center justify-center md:justify-start md:px-3">
          <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-primary-600 text-white font-bold text-sm">
            R
          </div>
          <span class="ml-2 hidden font-semibold md:inline">Raki</span>
        </div>

        <nav class="flex flex-1 flex-col gap-1">
          {navItems.map((item) => {
            const isActive = location.pathname.startsWith(item.href);
            const Icon = item.icon;
            return (
              <A
                href={item.href}
                class={cn(
                  "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                  isActive
                    ? "bg-primary-50 text-primary-700 dark:bg-primary-950 dark:text-primary-300"
                    : "text-muted-foreground hover:bg-muted hover:text-foreground"
                )}
                end={item.href === "/"}
                preload
              >
                <Icon class="h-5 w-5 shrink-0" />
                <span class="hidden md:inline">{item.label}</span>
              </A>
            );
          })}
        </nav>

        {/* Theme toggle */}
        <div class="mt-auto border-t border-border pt-3">
          <button
            type="button"
            onClick={() => theme.toggle()}
            class="flex w-full items-center gap-3 rounded-md px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label="Toggle theme"
          >
            <Switch fallback={<Laptop class="h-5 w-5" />}>
              <Match when={theme.resolvedTheme() === "dark"}>
                <Moon class="h-5 w-5" />
              </Match>
              <Match when={theme.resolvedTheme() === "light"}>
                <Sun class="h-5 w-5" />
              </Match>
            </Switch>
            <span class="hidden md:inline">Theme</span>
          </button>
        </div>
      </aside>

      {/* Main content */}
      <main class="flex-1 overflow-auto">{props.children}</main>
    </div>
  );
}
