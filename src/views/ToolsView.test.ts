import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";
import { snapshotWithTools, updatePlan } from "../test/fixtures";
import { apiError, createMockTransport, ok } from "../test/mockTransport";
import { setTransportForTests } from "../api/transport";
import { useSnapshotStore } from "../stores/snapshot";
import { useRunsStore } from "../stores/runs";
import { mapToolRecords } from "../utils/presentation";
import ToolCard from "../components/ToolCard.vue";
import ToolsView from "./ToolsView.vue";

function testRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/tools", name: "tools", component: ToolsView },
      { path: "/runs/confirm", name: "confirm", component: { template: "<div>confirm</div>" } },
      { path: "/runs", name: "runs", component: { template: "<div>runs</div>" } },
    ],
  });
}

describe("ToolsView", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("shows read-only release updates without update actions or an empty strategy selector", async () => {
    const snapshot = snapshotWithTools(1);
    snapshot.tools[0]!.displayName = "DashBeam";
    snapshot.installations[0]!.providerId = "github-releases";
    const component = snapshot.tools[0]!.components[0]!;
    component.installedVersion = "0.7.0";
    component.latestVersion = "0.7.1";
    component.strategies = [];
    component.defaultStrategy = null;
    const mock = createMockTransport({ snapshot: vi.fn(() => ok(snapshot)) });
    setTransportForTests(mock.transport);
    useSnapshotStore().acceptSnapshot(snapshot);
    const router = testRouter();
    await router.push("/tools");
    const wrapper = mount(ToolsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();

    expect(wrapper.get(".tool-card__version").text()).toBe("0.7.0 → 0.7.1");
    expect(wrapper.get(".tool-card .status-pill").text()).toBe("有更新");
    expect(wrapper.find(".tool-card__update").exists()).toBe(false);
    expect(wrapper.find('.tool-card input[type="checkbox"]').exists()).toBe(false);
    expect(wrapper.get('button[title="直接更新当前筛选结果中的所有可更新项"]').attributes("disabled")).toBeDefined();
    await wrapper.get(".tool-card__summary").trigger("click");
    const detail = wrapper.get('[role="dialog"]');
    expect(detail.text()).toContain("已安装 0.7.0 · 可用版本 0.7.1");
    expect(detail.text()).toContain("仅检查更新");
    expect(detail.find("select").exists()).toBe(false);
    expect(mock.transport.commands.preview).not.toHaveBeenCalled();
    wrapper.unmount();
  });

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

  it("supports filters, hidden state and keyboard focus", async () => {
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
    await wrapper.get('input[name="tool-status"][value="updates"]').setValue(true);

    expect(wrapper.text()).toContain("1 / 2 项");
    expect(wrapper.text()).toContain("Shared CLI");
    const updateSelection = wrapper.get('input[aria-label*="全部更新"]');
    await updateSelection.setValue(true);
    expect(wrapper.get(".selection-bar").text()).toContain("已选择 1 个 Component");
    await updateSelection.setValue(false);
    expect(wrapper.find(".selection-bar").exists()).toBe(false);
    wrapper.unmount();
  });

  it("keeps non-search filters in the sidebar and includes registered sources without installations", async () => {
    const snapshot = snapshotWithTools(3);
    snapshot.providers.push({ providerId: "mise", enabled: true, status: null, errors: [], refreshedAt: snapshot.refreshedAt! });
    const mock = createMockTransport({ snapshot: vi.fn(() => Promise.resolve({ status: "ok", data: snapshot })) });
    setTransportForTests(mock.transport);
    useSnapshotStore().acceptSnapshot(snapshot);
    const router = testRouter();
    await router.push("/tools");
    const wrapper = mount(ToolsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();

    const sidebar = wrapper.get('aside[aria-label="工具筛选"]');
    expect(sidebar.find('input[type="search"]').exists()).toBe(false);
    expect(sidebar.text()).toContain("类别");
    expect(sidebar.text()).toContain("显示已隐藏");
    expect(sidebar.text()).toContain("mise");
    expect(wrapper.get(".tool-search").findAll("input")).toHaveLength(1);
    await sidebar.get('input[name="tool-provider"][value="mise"]').setValue(true);
    expect(wrapper.text()).toContain("0 / 3 项");
    expect(sidebar.get('input[name="tool-provider"][value="mise"]').exists()).toBe(true);
    await sidebar.get("button").trigger("click");
    expect(wrapper.text()).toContain("3 / 3 项");
    expect(wrapper.findAll(".source-badge").map((badge) => badge.text())).toContain("npm");
    wrapper.unmount();
  });

  it("updates only eligible visible tools in one click without visiting confirmation", async () => {
    const snapshot = snapshotWithTools(10);
    snapshot.tools[0]!.hidden = true;
    snapshot.tools[9]!.components[0]!.strategies[0]!.enabled = false;
    const mock = createMockTransport({
      snapshot: vi.fn(() => ok(snapshot)),
      preview: vi.fn(() => ok([updatePlan()])),
      confirmBatch: vi.fn(() => ok({ outcomes: [], snapshot })),
    });
    setTransportForTests(mock.transport);
    useSnapshotStore().acceptSnapshot(snapshot);
    const router = testRouter();
    await router.push("/tools");
    const wrapper = mount(ToolsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();
    const button = wrapper.get('button[title="直接更新当前筛选结果中的所有可更新项"]');
    expect(button.text()).toContain("2 项");
    await wrapper.get('input[name="tool-provider"][value="npm"]').setValue(true);
    expect(button.text()).toContain("1 项");
    await button.trigger("click");
    await flushPromises();

    expect(mock.transport.commands.preview).toHaveBeenCalledExactlyOnceWith({ selections: [{
      toolId: "npm:tool-6", componentId: "core", strategyId: "provider-default", targetVersion: null,
    }] });
    expect(mock.transport.commands.confirmBatch).toHaveBeenCalledTimes(1);
    expect(router.currentRoute.value.name).toBe("runs");
    wrapper.unmount();
  });

  it("updates an individual tool without selecting it first", async () => {
    const mock = createMockTransport({
      preview: vi.fn(() => ok([updatePlan()])),
      confirmBatch: vi.fn(() => ok({ outcomes: [], snapshot: snapshotWithTools() })),
    });
    setTransportForTests(mock.transport);
    const router = testRouter();
    await router.push("/tools");
    const wrapper = mount(ToolsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();
    await wrapper.get('button[aria-label="更新 Shared CLI"]').trigger("click");
    await flushPromises();
    expect(mock.transport.commands.preview).toHaveBeenCalledExactlyOnceWith({ selections: [{
      toolId: "npm:tool-0", componentId: "core", strategyId: "provider-default", targetVersion: null,
    }] });
    expect(mock.transport.commands.confirmBatch).toHaveBeenCalledTimes(1);
    expect(router.currentRoute.value.name).toBe("runs");
    wrapper.unmount();
  });

  it("keeps the selected-update preview available and surfaces preparation failures", async () => {
    const mock = createMockTransport({ preview: vi.fn(() => apiError("unavailable", "更新服务暂不可用")) });
    setTransportForTests(mock.transport);
    const router = testRouter();
    await router.push("/tools");
    const wrapper = mount(ToolsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();
    await wrapper.get('input[aria-label*="全部更新"]').setValue(true);
    expect(wrapper.get(".selection-bar").text()).toContain("更新所选");
    const preview = wrapper.get(".selection-bar .button.secondary");
    await preview.trigger("click");
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain("更新服务暂不可用");
    expect(router.currentRoute.value.name).toBe("tools");
    expect(mock.transport.commands.confirmBatch).not.toHaveBeenCalled();
    useRunsStore().startingUpdate = true;
    await flushPromises();
    expect(wrapper.get(".tool-card__update").attributes("disabled")).toBeDefined();
    expect(preview.attributes("disabled")).toBeDefined();
    wrapper.unmount();
  });

  it("shows completed source counts while probing a Provider", async () => {
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
      stage: "probe",
      packageName: null,
      itemCompleted: null,
      itemTotal: null,
    });
    const router = testRouter();
    await router.push("/tools");
    await router.isReady();

    const wrapper = mount(ToolsView, {
      attachTo: document.body,
      global: { plugins: [router] },
    });
    await flushPromises();

    expect(wrapper.get(".refresh-progress").text()).toContain("正在检查 npm");
    expect(wrapper.get(".refresh-progress__count").text()).toBe("1 / 2");
    const progressbar = wrapper.get('[role="progressbar"]');
    expect(progressbar.attributes("aria-valuenow")).toBe("1");
    expect(progressbar.attributes("aria-valuemax")).toBe("2");
    wrapper.unmount();
  });

  it("shows real software progress and uses batch messaging when individual results are unavailable", async () => {
    setTransportForTests(createMockTransport().transport);
    const store = useSnapshotStore();
    store.acceptSnapshot(snapshotWithTools());
    store.refreshing = true;
    const event = {
      requestId: "refresh-1", sequence: "1", phase: "progress" as const, message: "checking node",
      providerId: "mise", completed: 1, total: 3, stage: "check_updates" as const,
      packageName: "node", itemCompleted: 2, itemTotal: 5,
    };
    store.applyRefreshProgress(event);
    const router = testRouter();
    await router.push("/tools");
    const wrapper = mount(ToolsView, { attachTo: document.body, global: { plugins: [router] } });
    await flushPromises();
    expect(wrapper.get(".refresh-progress").text()).toContain("检查更新 · node");
    expect(wrapper.get(".refresh-progress__count").text()).toBe("2 / 5");
    expect(wrapper.get('[role="progressbar"]').attributes("aria-valuenow")).toBe("2");
    expect(wrapper.get('[role="progressbar"]').attributes("aria-valuemax")).toBe("5");
    expect(wrapper.get('[role="progressbar"] > span').attributes("style")).toContain("40%");
    store.applyRefreshProgress({ ...event, sequence: "2", packageName: null, itemCompleted: 0 });
    await flushPromises();
    expect(wrapper.get(".refresh-progress").text()).toContain("正在批量查询版本");
    expect(wrapper.get('[role="progressbar"] > span').attributes("style")).toContain("0%");
    wrapper.unmount();
  });
});
