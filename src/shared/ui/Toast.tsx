import { AlertCircle, AlertTriangle, CheckCircle, Info, X } from "lucide-solid";
import {
  createContext,
  createSignal,
  For,
  type ParentComponent,
  useContext,
} from "solid-js";
import { cn } from "~/shared/lib/cn";

export type ToastType = "success" | "error" | "warning" | "info";

export interface ToastItem {
  id: string;
  type: ToastType;
  title?: string;
  message: string;
  duration?: number;
}

interface ToastContextValue {
  add: (toast: Omit<ToastItem, "id">) => void;
  remove: (id: string) => void;
}

const ToastContext = createContext<ToastContextValue>();

let toastIdCounter = 0;

export const ToastProvider: ParentComponent = (props) => {
  const [toasts, setToasts] = createSignal<ToastItem[]>([]);

  const add = (toast: Omit<ToastItem, "id">) => {
    const id = `toast-${++toastIdCounter}`;
    const duration = toast.duration ?? 5000;
    const item: ToastItem = { ...toast, id, duration };
    setToasts((prev) => [...prev, item]);

    if (duration > 0) {
      setTimeout(() => remove(id), duration);
    }
  };

  const remove = (id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  };

  const icons: Record<ToastType, typeof CheckCircle> = {
    success: CheckCircle,
    error: AlertCircle,
    warning: AlertTriangle,
    info: Info,
  };

  const styles: Record<ToastType, string> = {
    success:
      "border-success-200 bg-success-50 text-success-800 dark:border-success-900 dark:bg-success-950 dark:text-success-200",
    error:
      "border-error-200 bg-error-50 text-error-800 dark:border-error-900 dark:bg-error-950 dark:text-error-200",
    warning:
      "border-warning-200 bg-warning-50 text-warning-800 dark:border-warning-900 dark:bg-warning-950 dark:text-warning-200",
    info: "border-primary-200 bg-primary-50 text-primary-800 dark:border-primary-900 dark:bg-primary-950 dark:text-primary-200",
  };

  return (
    <ToastContext.Provider value={{ add, remove }}>
      {props.children}
      {/* Toast portal */}
      <section
        class="fixed bottom-4 right-4 z-[100] flex flex-col gap-2"
        aria-live="polite"
        aria-label="Notifications"
      >
        <For each={toasts()}>
          {(toast) => {
            const Icon = icons[toast.type];
            return (
              <div
                class={cn(
                  "pointer-events-auto flex w-80 items-start gap-3 rounded-lg border p-4 shadow-toast animate-slide-up",
                  styles[toast.type]
                )}
                role="alert"
              >
                <Icon class="mt-0.5 h-5 w-5 shrink-0" />
                <div class="flex-1 min-w-0">
                  {toast.title && (
                    <p class="font-semibold text-sm">{toast.title}</p>
                  )}
                  <p class="text-sm">{toast.message}</p>
                </div>
                <button
                  type="button"
                  onClick={() => remove(toast.id)}
                  class="shrink-0 rounded p-0.5 hover:bg-black/5 dark:hover:bg-white/10"
                  aria-label="Dismiss notification"
                >
                  <X class="h-4 w-4" />
                </button>
              </div>
            );
          }}
        </For>
      </section>
    </ToastContext.Provider>
  );
};

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within a ToastProvider");
  return ctx;
}
