import { defineStore } from "pinia";
import { ref } from "vue";

export type NotificationTone = "info" | "success" | "warning" | "error";

export type AppNotification = {
  id: number;
  tone: NotificationTone;
  title: string;
  message: string;
};

export const useNotificationsStore = defineStore("notifications", () => {
  const items = ref<AppNotification[]>([]);
  const dismissTimers = new Map<number, ReturnType<typeof setTimeout>>();
  let nextId = 1;

  const autoDismissDelay: Record<NotificationTone, number> = {
    info: 5_000,
    success: 5_000,
    warning: 8_000,
    error: 0,
  };

  function push(
    tone: NotificationTone,
    title: string,
    message = "",
    duration = autoDismissDelay[tone],
  ): number {
    const id = nextId++;
    items.value.push({ id, tone, title, message });
    if (duration > 0) {
      dismissTimers.set(
        id,
        setTimeout(() => dismiss(id), duration),
      );
    }
    return id;
  }

  function dismiss(id: number): void {
    const timer = dismissTimers.get(id);
    if (timer) clearTimeout(timer);
    dismissTimers.delete(id);
    items.value = items.value.filter((item) => item.id !== id);
  }

  return { items, push, dismiss };
});
