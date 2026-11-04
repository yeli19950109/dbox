import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTransportForTests } from "../api/transport";
import type { CodexUsageReport } from "../bindings";
import { apiError, createMockTransport, ok } from "../test/mockTransport";
import UsageView from "./UsageView.vue";

function usage(requests: number, input: number, cached: number, output: number) {
  return {
    requests,
    inputTokens: input,
    cachedInputTokens: cached,
    cacheWriteInputTokens: 0,
    outputTokens: output,
    totalTokens: input + output,
  };
}

const report: CodexUsageReport = {
  codexHome: "/home/test/.codex",
  available: true,
  generatedAt: "2026-07-10T12:00:00Z",
  days: 30,
  filesScanned: 3,
  sessions: 2,
  totals: usage(3, 1_500, 1_000, 150),
  models: [
    { model: "gpt-5.5", usage: usage(1, 1_000, 800, 100) },
    { model: "gpt-5.4", usage: usage(2, 500, 200, 50) },
  ],
  daily: [
    { date: "2026-07-09", usage: usage(1, 500, 200, 50) },
    { date: "2026-07-10", usage: usage(2, 1_000, 800, 100) },
  ],
  deferred: [{ path: "/home/test/.codex/sessions/child.jsonl", reason: "找不到父会话: x" }],
  replayedRequests: 4,
  duplicateRequests: 1,
};

describe("UsageView", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("loads the 30-day window and renders totals, models and skipped files", async () => {
    const codexUsage = vi.fn(() => ok(report));
    const mock = createMockTransport({ codexUsage });
    setTransportForTests(mock.transport);
    const wrapper = mount(UsageView);
    await flushPromises();

    expect(codexUsage).toHaveBeenCalledWith({ days: 30 });
    expect(wrapper.get('[data-testid="usage-requests"]').text()).toBe("3");
    expect(wrapper.get('[data-testid="usage-total"]').text()).toBe("1,650");
    const models = wrapper.findAll("tbody tr").map((row) => row.get("th").text());
    expect(models).toEqual(["gpt-5.5", "gpt-5.4"]);
    expect(wrapper.text()).toContain("1 个会话文件未计入");
    expect(wrapper.text()).toContain("67%");

    await wrapper.findAll(".extension-tabs button").find((b) => b.text() === "全部")!.trigger("click");
    await flushPromises();
    expect(codexUsage).toHaveBeenLastCalledWith({ days: null });
    wrapper.unmount();
  });

  it("explains a missing Codex directory and surfaces backend errors", async () => {
    const mock = createMockTransport();
    setTransportForTests(mock.transport);
    const wrapper = mount(UsageView);
    await flushPromises();
    expect(wrapper.text()).toContain("未找到 Codex 目录");

    mock.transport.commands.codexUsage = vi.fn(() => apiError("internal", "扫描失败"));
    await wrapper.get(".extension-actions button").trigger("click");
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toBe("扫描失败");
    wrapper.unmount();
  });
});
