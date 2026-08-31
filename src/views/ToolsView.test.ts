import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";
import { snapshotWithTools } from "../test/fixtures";
import { createMockTransport } from "../test/mockTransport";
import { setTransportForTests } from "../api/transport";
import { useSnapshotStore } from "../stores/snapshot";
import { mapToolRecords } from "../utils/presentation";
import ToolCard from "../components/ToolCard.vue";
import ToolsView from "./ToolsView.vue";

function testRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/tools", name: "tools", component: ToolsView },
      { path: "/runs/confirm", name: "confirm", component: { template: "<div>confirm</div>" } },
    ],
  });
}

describe("ToolsView", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("keeps a 500-item list DOM bounded with TanStack Virtual", async () => {
    const snapshot = snapshotWithTools(500);
    const records = mapToolRecords(snapshot);
    const OrdinaryList = defineComponent(() => () =>
      h(
        "div",
        records.map((record) =>
          h(ToolCard, { record, selected: new Set<string>() }),
        ),
      ),
    );
    const ordinaryStart = performance.now();
    const ordinary = mount(OrdinaryList);
    const ordinaryDuration = performance.now() - ordinaryStart;
    const ordinaryDom = ordinary.findAll(".tool-card").length;
    ordinary.unmount();
    const mock = createMockTransport({ snapshot: vi.fn(() => Promise.resolve({ status: "ok", data: snapshot })) });
    setTransportForTests(mock.transport);
    const store = useSnapshotStore();
    store.acceptSnapshot(snapshot);
    const router = testRouter();
    await router.push("/tools");
    await router.isReady();

    const virtualStart = performance.now();
    const wrapper = mount(ToolsView, {
      attachTo: document.body,
      global: { plugins: [router] },
    });
    await flushPromises();
    const virtualDuration = performance.now() - virtualStart;

    const rows = wrapper.findAll("[data-index]");
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(30);
    expect(rows.length).toBeLessThan(ordinaryDom / 10);
    expect(wrapper.text()).toContain("500 / 500 项");
    console.info(
      `[tool-list benchmark] ordinary=${ordinaryDuration.toFixed(1)}ms/${ordinaryDom} cards virtual=${virtualDuration.toFixed(1)}ms/${rows.length} cards`,
    );
    wrapper.unmount();
  });

  it("puts tools with updates first and preserves source order within each group", async () => {
    const snapshot = snapshotWithTools(3);
    snapshot.tools = [snapshot.tools[1]!, snapshot.tools[2]!, snapshot.tools[0]!];
    const mock = createMockTransport({
      snapshot: vi.fn(() => Promise.resolve({ status: "ok", data: snapshot })),
    });
    setTransportForTests(mock.transport);
    useSnapshotStore().acceptSnapshot(snapshot);
    const router = testRouter();
    await router.push("/tools");
    await router.isReady();

    const wrapper = mount(ToolsView, {
      attachTo: document.body,
      global: { plugins: [router] },
    });
    await flushPromises();

    expect(wrapper.findAll(".tool-card h2").map((heading) => heading.text())).toEqual([
      "Shared CLI",
      "Tool 1",
      "Tool 2",
    ]);
    wrapper.unmount();
  });

  it("supports filters, hidden state, keyboard focus and provider-scoped refresh", async () => {
    vi.useFakeTimers();
    const snapshot = snapshotWithTools(12);
    snapshot.tools[1]!.hidden = true;
    const mock = createMockTransport({ snapshot: vi.fn(() => Promise.resolve({ status: "ok", data: snapshot })) });
    setTransportForTests(mock.transport);
    useSnapshotStore().acceptSnapshot(snapshot);
    const router = testRouter();
    await router.push("/tools");
    await router.isReady();
    const wrapper = mount(ToolsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();

    const search = wrapper.get('input[type="search"]');
    await search.setValue("package-0");
    await vi.advanceTimersByTimeAsync(150);
    expect(wrapper.text()).toContain("1 / 12 项");

    await search.setValue("");
    await vi.advanceTimersByTimeAsync(150);
    const details = wrapper.get('button[aria-label*="查看"]');
    await details.trigger("click");
    await flushPromises();
    const close = wrapper.get('button[aria-label="关闭工具详情"]');
    expect(document.activeElement).toBe(close.element);
    await close.trigger("keydown", { key: "Escape" });

    const providerRefresh = wrapper.findAll("button").find((button) => button.text().includes("刷新 npm"));
    await providerRefresh!.trigger("click");
    expect(mock.transport.commands.refresh).toHaveBeenCalledWith({
      scope: { scope: "provider", providerId: "npm" },
      force: true,
    });
    wrapper.unmount();
  });

  it("includes direct update_available tools when hasUpdates is null", async () => {
    const snapshot = snapshotWithTools(2);
    snapshot.tools[0]!.status.hasUpdates = null;
    const mock = createMockTransport({
      snapshot: vi.fn(() => Promise.resolve({ status: "ok", data: snapshot })),
    });
    setTransportForTests(mock.transport);
    useSnapshotStore().acceptSnapshot(snapshot);
    const router = testRouter();
    await router.push("/tools");
    await router.isReady();

    const wrapper = mount(ToolsView, {
      attachTo: document.body,
      global: { plugins: [router] },
    });
    await flushPromises();
    await wrapper.findAll("select")[0]!.setValue("updates");

    expect(wrapper.text()).toContain("1 / 2 项");
    expect(wrapper.text()).toContain("Shared CLI");
    const updateSelection = wrapper.get('input[aria-label*="全部更新"]');
    await updateSelection.setValue(true);
    expect(wrapper.get(".selection-bar").text()).toContain("已选择 1 个 Component");
    await updateSelection.setValue(false);
    expect(wrapper.find(".selection-bar").exists()).toBe(false);
    wrapper.unmount();
  });

  it("shows Provider-level refresh progress", async () => {
    const mock = createMockTransport();
    setTransportForTests(mock.transport);
    const store = useSnapshotStore();
    store.acceptSnapshot(snapshotWithTools());
    store.refreshing = true;
    store.applyRefreshProgress({
      requestId: "refresh-1",
      sequence: "1",
      phase: "progress",
      message: "checking npm-global",
      providerId: "npm-global",
      completed: 1,
      total: 2,
    });
    const router = testRouter();
    await router.push("/tools");
    await router.isReady();

    const wrapper = mount(ToolsView, {
      attachTo: document.body,
      global: { plugins: [router] },
    });
    await flushPromises();

    expect(wrapper.get(".refresh-progress").text()).toContain("正在检查 npm-global");
    expect(wrapper.get(".refresh-progress").text()).toContain("2 / 2");
    const progressbar = wrapper.get('[role="progressbar"]');
    expect(progressbar.attributes("aria-valuenow")).toBe("1");
    expect(progressbar.attributes("aria-valuemax")).toBe("2");
    wrapper.unmount();
  });
});
