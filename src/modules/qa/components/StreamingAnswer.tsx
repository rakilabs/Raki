import { Bot } from "lucide-solid";
import { Skeleton } from "~/shared/ui";

export function StreamingAnswer() {
  return (
    <div class="flex gap-4">
      <div class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Bot class="h-4 w-4" />
      </div>
      <div class="max-w-[80%] space-y-2 rounded-xl border border-border bg-card px-4 py-3">
        <Skeleton width="80%" height="12px" />
        <Skeleton width="60%" height="12px" />
        <Skeleton width="40%" height="12px" />
      </div>
    </div>
  );
}
