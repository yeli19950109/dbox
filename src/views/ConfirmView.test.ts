import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import type { ConfirmResponseDto } from "../bindings";
import { setTransportForTests } from "../api/transport";
import { snapshotWithTools, updatePlan } from "../test/fixtures";
import { apiError, createMockTransport, ok } from "../test/mockTransport";
import { useRunsStore } from "../stores/runs";
import { useSnapshotStore } from "../stores/snapshot";
import ConfirmView from "./ConfirmView.vue";
import RunsView from "./RunsView.vue";

function testRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/tools", name: "tools", component: { template: "<div>tools</div>" } },
      { path: "/runs/confirm", name: "confirm", component: ConfirmView },
      { path: "/runs", name: "runs", component: RunsView },
    ],
  });
}

describe("confirmed execution live view", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("opens the run page and streams output while confirm is still pending", async () => {
    let finish!: (value: { status: "ok"; data: ConfirmResponseDto }) => void;
    const response = new Promise<{ status: "ok"; data: ConfirmResponseDto }>((resolve) => { finish = resolve; });
    const plan = updatePlan();
    const snapshot = snapshotWithTools();
    const mock = createMockTransport({ preview: vi.fn(() => ok([plan])), confirm: vi.fn(() => response) });
    setTransportForTests(mock.transport);
    useSnapshotStore().acceptSnapshot(snapshot);
    const runs = useRunsStore();
    await runs.preview([{ toolId: plan.toolId, componentId: "core", strategyId: null, targetVersion: null }]);
    const router = testRouter();
    await router.push("/runs/confirm");
    const wrapper = mount({ template: "<RouterView />" }, { attachTo: document.body, global: { plugins: [router] } });
    await wrapper.get("button.button.danger").trigger("click");
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("runs");
    expect(runs.executing).toBe(true);
    expect(wrapper.text()).toContain("正在启动更新");
    expect(mock.transport.commands.confirm).toHaveBeenCalledTimes(1);
    await runs.confirmPlans();
    expect(mock.transport.commands.confirm).toHaveBeenCalledTimes(1);
    mock.emit("runState", { runId: "live-run", sequence: "1", timestamp: "2026-09-07T10:00:00Z", state: { kind: "state_changed", status: "running" } });
    mock.emit("runOutput", { runId: "live-run", firstSequence: "2", lastSequence: "2", timestamp: "2026-09-07T10:00:01Z", chunks: [{ sequence: "2", stream: "stdout", message: "Downloading fixture..." }] });
    await flushPromises();
    expect(wrapper.get('[role="log"]').text()).toContain("Downloading fixture...");
    expect(wrapper.get('button[aria-label="取消运行 live-run"]').exists()).toBe(true);
    expect(runs.executing).toBe(true);

    mock.emit("runOutput", { runId: "live-run", firstSequence: "3", lastSequence: "3", timestamp: "2026-09-07T10:00:02Z", chunks: [{ sequence: "3", stream: "stderr", message: "Installing fixture..." }] });
    await flushPromises();
    expect(wrapper.get('[role="log"]').text()).toContain("Installing fixture...");
    finish({ status: "ok", data: {
      execution: { runId: "live-run", status: "succeeded", exitCode: 0, startedAt: "2026-09-07T10:00:00Z", finishedAt: "2026-09-07T10:00:03Z", outputTail: "finished", verification: null },
      snapshot,
    } });
    await flushPromises();
    expect(runs.executing).toBe(false);
    expect(wrapper.get(".run-detail__header").text()).toContain("成功");
    expect(wrapper.get('[role="log"]').text()).toContain("Downloading fixture...");
    wrapper.unmount();
    runs.disconnectEvents();
  });

  it("keeps expired-plan recovery available on the run page without executing again", async () => {
    const plan = updatePlan();
    const mock = createMockTransport({ preview: vi.fn(() => ok([plan])), confirm: vi.fn(() => apiError("invalid_plan", "expired")) });
    setTransportForTests(mock.transport);
    const runs = useRunsStore();
    await runs.preview([{ toolId: plan.toolId, componentId: "core", strategyId: null, targetVersion: null }]);
    const router = testRouter();
    await router.push("/runs/confirm");
    const wrapper = mount({ template: "<RouterView />" }, { attachTo: document.body, global: { plugins: [router] } });
    await wrapper.get("button.button.danger").trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.name).toBe("runs");
    expect(wrapper.get('[role="alert"]').text()).toContain("重新预览");
    await wrapper.get('[role="alert"] button').trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.name).toBe("confirm");
    expect(mock.transport.commands.confirm).toHaveBeenCalledTimes(1);
    wrapper.unmount();
    runs.disconnectEvents();
  });
});
