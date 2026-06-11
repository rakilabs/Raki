import { Check, Laptop, Moon, Sun } from "lucide-solid";
import { Show } from "solid-js";
import { useTheme } from "~/app/providers/ThemeProvider";
import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "~/shared/ui";

export function AppearanceTab() {
  const theme = useTheme();

  const options = [
    {
      value: "light" as const,
      label: "Light",
      icon: Sun,
      description: "Always use light mode",
    },
    {
      value: "dark" as const,
      label: "Dark",
      icon: Moon,
      description: "Always use dark mode",
    },
    {
      value: "system" as const,
      label: "System",
      icon: Laptop,
      description: "Follow system preference",
    },
  ];

  return (
    <Card>
      <CardHeader>
        <CardTitle>Appearance</CardTitle>
        <CardDescription>
          Customize how Raki looks on your device
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div class="grid gap-4 sm:grid-cols-3">
          {options.map((option) => {
            const isActive = theme.theme() === option.value;
            const Icon = option.icon;
            return (
              <Button
                variant={isActive ? "primary" : "outline"}
                class="flex h-auto flex-col items-start gap-2 p-4 text-left"
                onClick={() => theme.setTheme(option.value)}
              >
                <div class="flex w-full items-center justify-between">
                  <Icon class="h-5 w-5" />
                  <Show when={isActive}>
                    <Check class="h-4 w-4" />
                  </Show>
                </div>
                <div>
                  <p class="font-medium">{option.label}</p>
                  <p class="text-xs text-muted-foreground">
                    {option.description}
                  </p>
                </div>
              </Button>
            );
          })}
        </div>
      </CardContent>
    </Card>
  );
}
