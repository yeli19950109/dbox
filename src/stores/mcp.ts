import { resourceEvents } from "../utils/extensionEvents";
import { defineStore } from "pinia";
import { ref } from "vue";
import type {
  AgentTarget,
  McpSnapshot,
  McpScan,
  McpOperationRequest,
  McpEdit,
  ExtensionBackup,
} from "../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";
import { useExtensionOperations } from "./extensions";
export const useMcpStore = defineStore("mcp", () => {
  const snapshot = ref<McpSnapshot>({ revision: "missing", servers: [] }),
    targets = ref<AgentTarget[]>([]);
  const scan = ref<McpScan>({ candidates: [], errors: [] }),
    backups = ref<ExtensionBackup[]>([]);
  const loading = ref(false),
    error = ref<string | null>(null);
  let epoch = 0;
  async function load() {
    const generation = ++epoch;
    loading.value = true;
    try {
      const [s, t, b] = await Promise.all([
        unwrapCommand(getTransport().commands.listMcpServers()),
        unwrapCommand(getTransport().commands.listAgentTargets()),
        unwrapCommand(getTransport().commands.listSkillBackups()),
      ]);
      if (generation !== epoch) return;
      snapshot.value = s;
      targets.value = t;
      backups.value = b.filter((b) => b.resource === "mcp");
      error.value = null;
    } catch (e) {
      if (generation === epoch) error.value = errorMessage(e, "读取 MCP 失败");
    } finally {
      if (generation === epoch) loading.value = false;
    }
  }
  async function scanImports() {
    try {
      scan.value = await unwrapCommand(
        getTransport().commands.scanMcpImports(),
      );
      error.value = null;
    } catch (e) {
      error.value = errorMessage(e, "扫描失败");
    }
  }
  async function preview(
    action: McpOperationRequest["action"],
    options: {
      ids?: string[];
      candidates?: string[];
      targets?: string[];
      enabled?: boolean;
      edit?: McpEdit;
      importJson?: string;
      backupId?: string;
    } = {},
  ) {
    await useExtensionOperations().preview(
      () =>
        unwrapCommand(
          getTransport().commands.previewMcpOperation({
            action,
            serverIds: options.ids ?? [],
            candidateIds: options.candidates ?? [],
            targetIds: options.targets ?? [],
            enabled: options.enabled ?? true,
            edit: options.edit ?? null,
            importJson: options.importJson ?? null,
            backupId: options.backupId ?? null,
          }),
        ),
      "mcp",
    );
  }
  const subscriptions = resourceEvents("mcp", load);
  return {
    connectEvents: subscriptions.connect,
    disconnectEvents: subscriptions.disconnect,
    snapshot,
    targets,
    scan,
    backups,
    loading,
    error,
    load,
    scanImports,
    preview,
  };
});
