import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import axe from "axe-core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { setTransportForTests } from "../api/transport";
import { failedRun } from "../test/fixtures";
import { createMockTransport, ok } from "../test/mockTransport";
import RunsView from "./RunsView.vue";

describe("RunsView accessibility", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("provides accessible retry/copy operations and an axe-clean status region", async () => {
    const run = failedRun();
    const mock = createMockTransport({
      runHistory: vi.fn(() => ok({ runs: [run] })),
      runLog: vi.fn(() =>
        ok({
          runId: run.id,
          entries: [
            { sequence: "1", timestamp: run.startedAt!, stream: "stderr", message: "redacted" },
          ],
        }),
      ),
    });
    setTransportForTests(mock.transport);
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/runs", component: RunsView },
        { path: "/runs/confirm", name: "confirm", component: { template: "<div>confirm</div>" } },
      ],
    });
    await router.push("/runs");
    await router.isReady();
    const wrapper = mount(RunsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();

    expect(wrapper.get('button[aria-label^="复制"]').attributes("aria-label")).toContain("脱敏日志");
    expect(wrapper.get('button[aria-label^="重试"]').attributes("aria-label")).toContain("重试");
    expect(wrapper.text()).toContain("更新完成，但版本检查结果未知");
    const results = await axe.run(wrapper.element, {
      rules: { "color-contrast": { enabled: true } },
    });
    expect(results.violations).toEqual([]);
    wrapper.unmount();
  });
});
