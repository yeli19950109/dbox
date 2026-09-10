import { resourceEvents } from "../utils/extensionEvents";
import { defineStore } from "pinia";
import { ref } from "vue";
import type {
  AgentTarget,
  SkillCandidate,
  SkillUpdate,
  SkillSource,
  SkillsSnapshot,
  Diagnostic,
  ExtensionBackup,
  DeployMode,
  SkillAction,
  SearchSkill,
} from "../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";
import { useExtensionOperations } from "./extensions";
export const useSkillsStore = defineStore("skills", () => {
  const snapshot = ref<SkillsSnapshot>({
    revision: "missing",
    sources: [],
    skills: [],
  });
  const targets = ref<AgentTarget[]>([]),
    candidates = ref<SkillCandidate[]>([]),
    imports = ref<SkillCandidate[]>([]);
  const updates = ref<SkillUpdate[]>([]),
    diagnostics = ref<Diagnostic[]>([]),
    backups = ref<ExtensionBackup[]>([]),
    searchResults = ref<SearchSkill[]>([]);
  const loading = ref(false),
    discovering = ref(false),
    error = ref<string | null>(null),
    requestId = ref<string | null>(null);
  let epoch = 0;
  async function load() {
    const generation = ++epoch;
    loading.value = true;
    try {
      const [s, t, b] = await Promise.all([
        unwrapCommand(getTransport().commands.listSkills()),
        unwrapCommand(getTransport().commands.listAgentTargets()),
        unwrapCommand(getTransport().commands.listSkillBackups()),
      ]);
      if (generation !== epoch) return;
      snapshot.value = s;
      targets.value = t;
      backups.value = b.filter((b) => b.resource === "skill");
      error.value = null;
    } catch (e) {
      if (generation === epoch)
        error.value = errorMessage(e, "读取 Skills 失败");
    } finally {
      if (generation === epoch) loading.value = false;
    }
  }
  async function task<T>(
    work: (id: string) => Promise<T>,
  ): Promise<T | undefined> {
    if (discovering.value) return;
    discovering.value = true;
    error.value = null;
    const id = crypto.randomUUID();
    requestId.value = id;
    try {
      return await work(id);
    } catch (e) {
      error.value = errorMessage(e, "请求失败");
    } finally {
      discovering.value = false;
      requestId.value = null;
    }
  }
  async function discover(source: SkillSource) {
    await task(async (id) => {
      const d = await unwrapCommand(
        getTransport().commands.discoverSkills({ requestId: id, source }),
      );
      candidates.value = d.candidates;
      diagnostics.value = d.errors;
    });
  }
  async function scanImports() {
    await task(async () => {
      const d = await unwrapCommand(getTransport().commands.scanSkillImports());
      imports.value = d.candidates;
      diagnostics.value = d.errors;
    });
  }
  async function check(ids: string[]) {
    await task(async (id) => {
      updates.value = await unwrapCommand(
        getTransport().commands.checkSkillUpdates({
          requestId: id,
          skillIds: ids,
        }),
      );
    });
  }
  async function search(query: string) {
    await task(async (id) => {
      searchResults.value = await unwrapCommand(
        getTransport().commands.searchSkills({ requestId: id, query }),
      );
    });
  }
  async function saveSource(source: SkillSource) {
    await task(async () => {
      snapshot.value = await unwrapCommand(
        getTransport().commands.saveSkillSource({
          source,
          expectedRevision: snapshot.value.revision,
        }),
      );
    });
  }
  async function deleteSource(id: string) {
    await task(async () => {
      snapshot.value = await unwrapCommand(
        getTransport().commands.deleteSkillSource({
          sourceId: id,
          expectedRevision: snapshot.value.revision,
        }),
      );
    });
  }
  async function preview(
    action: SkillAction,
    options: {
      ids?: string[];
      candidates?: string[];
      targets?: string[];
      enabled?: boolean;
      mode?: DeployMode;
      backupId?: string;
    } = {},
  ) {
    await useExtensionOperations().preview(
      () =>
        unwrapCommand(
          getTransport().commands.previewSkillOperation({
            action,
            skillIds: options.ids ?? [],
            candidateIds: options.candidates ?? [],
            targetIds: options.targets ?? [],
            enabled: options.enabled ?? true,
            mode: options.mode ?? "auto",
            backupId: options.backupId ?? null,
          }),
        ),
      "skill",
    );
  }
  async function cancelRequest() {
    if (requestId.value)
      await unwrapCommand(
        getTransport().commands.cancel({ runId: requestId.value }),
      ).catch(() => undefined);
  }
  const subscriptions = resourceEvents("skill", load);
  return {
    connectEvents: subscriptions.connect,
    disconnectEvents: subscriptions.disconnect,
    snapshot,
    targets,
    candidates,
    imports,
    updates,
    diagnostics,
    backups,
    searchResults,
    loading,
    discovering,
    error,
    load,
    discover,
    scanImports,
    check,
    search,
    saveSource,
    deleteSource,
    preview,
    cancelRequest,
  };
});
