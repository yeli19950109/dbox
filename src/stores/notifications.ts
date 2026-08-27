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
  let nextId = 1;

  function push(
    tone: NotificationTone,
    title: string,
    message = "",
  ): number {
    const id = nextId++;
    items.value.push({ id, tone, title, message });
    return id;
  }

  function dismiss(id: number): void {
    items.value = items.value.filter((item) => item.id !== id);
  }

  return { items, push, dismiss };
});
