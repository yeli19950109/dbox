import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import SkillsView from "./SkillsView.vue";
import McpView from "./McpView.vue";
import McpEditor from "../components/mcp/McpEditor.vue";
import { createMockTransport, ok, apiError } from "../test/mockTransport";
import { setTransportForTests } from "../api/transport";
import type {
  AgentTarget,
  ExtensionPlan,
  SkillRecord,
  McpServerView,
} from "../bindings";
import { useExtensionOperations } from "../stores/extensions";
import { useSkillsStore } from "../stores/skills";
const targets: AgentTarget[] = ["claude", "codex", "gemini"].map((id) => ({
  id,
  name: id,
  scope: "user",
  skillsDir: `/fixture/${id}/skills`,
  mcpFile: `/fixture/${id}/config`,
  pathSource: "default",
  available: true,
  transports: id === "codex" ? ["stdio", "http"] : ["stdio", "http", "sse"],
  sharedWith: [],
  scanDirs: [],
}));
const plan: ExtensionPlan = {
  planId: "plan",
  planHash: "hash",
  resource: "skill",
  operation: "toggle",
  names: ["alpha"],
  expiresAt: "2099-01-01T00:00:00Z",
  steps: [],
  warnings: [],
};
function skill(id: string, name: string): SkillRecord {
  return {
    id,
    name,
    description: "",
    directoryName: name,
    origin: null,
    contentHash: "hash",
    updatedAt: "2026-09-10T00:00:00Z",
    deployments: [
      {
        targetId: "claude",
        path: `/fixture/${name}`,
        desiredEnabled: true,
        mode: "copy",
        lastAppliedHash: "hash",
        observedState: "enabled",
      },
    ],
    pendingDelete: false,
  };
}
async function render(component: typeof SkillsView | typeof McpView) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component },
      { path: "/runs", component: { template: "<div>runs</div>" } },
    ],
  });
  await router.push("/");
  return mount(component, { global: { plugins: [router] } });
}
beforeEach(() => setActivePinia(createPinia()));
describe("Skill management view", () => {
  it("limits bulk mutations to selected items in the current filter", async () => {
    const preview = vi.fn(() => ok(plan));
    const { transport } = createMockTransport({
      listSkills: () =>
        ok({
          revision: "one",
          sources: [],
          skills: [skill("1", "alpha"), skill("2", "beta")],
        }),
      listAgentTargets: () => ok(targets),
      previewSkillOperation: preview,
    });
    setTransportForTests(transport);
    const wrapper = await render(SkillsView);
    await flushPromises();
    await wrapper.get('input[aria-label="选择 alpha"]').setValue(true);
    await wrapper.get('input[aria-label="选择 beta"]').setValue(true);
    await wrapper.get('input[type="search"]').setValue("alpha");
    const remove = wrapper
      .findAll("button")
      .find((b) => b.text() === "卸载选中项")!;
    await remove.trigger("click");
    await flushPromises();
    expect(preview).toHaveBeenCalledWith(
      expect.objectContaining({ skillIds: ["1"], action: "uninstall" }),
    );
    expect(wrapper.text()).toContain("确认执行");
    wrapper.unmount();
  });
  it("offers explicit adoption for external installations", async () => {
    const s = skill("1", "external");
    s.deployments[0]!.mode = "external";
    s.deployments[0]!.observedState = "external";
    const preview = vi.fn(() => ok(plan));
    const { transport } = createMockTransport({
      listSkills: () => ok({ revision: "one", sources: [], skills: [s] }),
      listAgentTargets: () => ok(targets),
      previewSkillOperation: preview,
    });
    setTransportForTests(transport);
    const wrapper = await render(SkillsView);
    await flushPromises();
    await wrapper
      .findAll("button")
      .find((b) => b.text() === "预览接管")!
      .trigger("click");
    await flushPromises();
    expect(preview).toHaveBeenCalledWith(
      expect.objectContaining({ action: "adopt", targetIds: ["claude"] }),
    );
    wrapper.unmount();
  });
  it("refreshes authority after a failed or partial operation", async () => {
    const list = vi.fn(() => ok({ revision: "one", sources: [], skills: [] }));
    const { transport } = createMockTransport({ listSkills: list });
    setTransportForTests(transport);
    const wrapper = await render(SkillsView);
    await flushPromises();
    const ops = useExtensionOperations();
    ops.pending = true;
    await flushPromises();
    ops.result = {
      runId: "run",
      status: "partial",
      targets: [],
      backupId: null,
    };
    ops.pending = false;
    await flushPromises();
    expect(list).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });
  it("keeps the newer snapshot when earlier requests arrive late", async () => {
    let resolve!: (
      value: Awaited<ReturnType<typeof transport.commands.listSkills>>,
    ) => void;
    const first = new Promise<
      Awaited<ReturnType<typeof transport.commands.listSkills>>
    >((r) => (resolve = r));
    const { transport } = createMockTransport({
      listSkills: vi
        .fn()
        .mockReturnValueOnce(first)
        .mockReturnValueOnce(ok({ revision: "new", sources: [], skills: [] })),
    });
    setTransportForTests(transport);
    const store = useSkillsStore();
    const old = store.load();
    await store.load();
    resolve({
      status: "ok",
      data: { revision: "old", sources: [], skills: [] },
    });
    await old;
    expect(store.snapshot.revision).toBe("new");
  });
});
describe("MCP management view", () => {
  const server: McpServerView = {
    id: "server",
    name: "alpha",
    description: "safe",
    transport: "http",
    configJson: '{"url":"[REDACTED]","headers":{"Authorization":"[REDACTED]"}}',
    bindings: [
      {
        targetId: "codex",
        key: "alpha",
        desiredEnabled: true,
        observedState: "conflict",
        extraJson: "{}",
      },
    ],
    pendingDelete: false,
  };
  it("searches names and descriptions without indexing config", async () => {
    const { transport } = createMockTransport({
      listMcpServers: () => ok({ revision: "one", servers: [server] }),
      listAgentTargets: () => ok(targets),
    });
    setTransportForTests(transport);
    const wrapper = await render(McpView);
    await flushPromises();
    expect(wrapper.text()).toContain("alpha");
    await wrapper.get('input[type="search"]').setValue("Authorization");
    expect(wrapper.text()).toContain("暂无匹配的服务器");
    wrapper.unmount();
  });
  it("retries only the selected failed application", async () => {
    const preview = vi.fn(() => ok({ ...plan, resource: "mcp" as const }));
    const { transport } = createMockTransport({
      listMcpServers: () => ok({ revision: "one", servers: [server] }),
      listAgentTargets: () => ok(targets),
      previewMcpOperation: preview,
    });
    setTransportForTests(transport);
    const wrapper = await render(McpView);
    await flushPromises();
    await wrapper
      .findAll("button")
      .find((b) => b.text() === "重试此应用")!
      .trigger("click");
    await flushPromises();
    expect(preview).toHaveBeenCalledWith(
      expect.objectContaining({
        action: "sync",
        serverIds: ["server"],
        targetIds: ["codex"],
      }),
    );
    wrapper.unmount();
  });
  it("uses keep/set/remove editing without putting real secrets in ordinary DTOs", async () => {
    const wrapper = mount(McpEditor, {
      props: { server, targets, disabled: false },
    });
    const selects = wrapper.findAll(".secret-edits select");
    await selects[0]!.setValue("keep");
    await selects[1]!.setValue("set");
    await wrapper.get('input[type="password"]').setValue("replacement");
    await wrapper.get("form").trigger("submit");
    const edit = wrapper.emitted("save")![0]![0] as {
      secrets: Record<string, unknown>;
    };
    expect(edit.secrets["/url"]).toEqual({ action: "keep" });
    expect(edit.secrets["/headers/Authorization"]).toEqual({
      action: "set",
      value: "replacement",
    });
    wrapper.unmount();
  });
  it("shows backend conflicts without optimistic enablement", async () => {
    const { transport } = createMockTransport({
      listMcpServers: () => ok({ revision: "one", servers: [server] }),
      listAgentTargets: () => ok(targets),
      previewMcpOperation: () => apiError("conflict", "外部配置已变化"),
    });
    setTransportForTests(transport);
    const wrapper = await render(McpView);
    await flushPromises();
    await wrapper
      .findAll("button")
      .find((b) => b.text() === "手动同步")!
      .trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("外部配置已变化");
    expect(wrapper.text()).toContain("conflict");
    wrapper.unmount();
  });
});
