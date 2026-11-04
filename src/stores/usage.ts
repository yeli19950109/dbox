import { defineStore } from "pinia";
import { ref } from "vue";
import type { CodexUsageReport } from "../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";

/** Day windows offered in the usage view; null means all history. */
export const USAGE_WINDOWS = [7, 30, 90, null] as const;
export type UsageWindow = (typeof USAGE_WINDOWS)[number];

export const useUsageStore = defineStore("usage", () => {
  const report = ref<CodexUsageReport | null>(null);
  const days = ref<UsageWindow>(30);
  const loading = ref(false);
  const error = ref<string | null>(null);
  let epoch = 0;

  async function load(window: UsageWindow = days.value) {
    const generation = ++epoch;
    days.value = window;
    loading.value = true;
    try {
      const result = await unwrapCommand(
        getTransport().commands.codexUsage({ days: window }),
      );
      if (generation !== epoch) return;
      report.value = result;
      error.value = null;
    } catch (reason) {
      if (generation === epoch) error.value = errorMessage(reason, "读取 Codex 用量失败");
    } finally {
      if (generation === epoch) loading.value = false;
    }
  }

  return { report, days, loading, error, load };
});
