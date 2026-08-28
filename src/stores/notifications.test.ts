import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useNotificationsStore } from "./notifications";

describe("notifications store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });

  afterEach(() => vi.useRealTimers());

  it("automatically dismisses success notifications", () => {
    const store = useNotificationsStore();
    store.push("success", "刷新完成");

    vi.advanceTimersByTime(4_999);
    expect(store.items).toHaveLength(1);

    vi.advanceTimersByTime(1);
    expect(store.items).toHaveLength(0);
  });

  it("keeps errors until they are dismissed", () => {
    const store = useNotificationsStore();
    const id = store.push("error", "刷新失败");

    vi.advanceTimersByTime(60_000);
    expect(store.items).toHaveLength(1);

    store.dismiss(id);
    expect(store.items).toHaveLength(0);
  });
});
