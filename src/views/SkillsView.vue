<template>
  <div class="view extension-view">
    <header class="extension-heading">
      <div>
        <p class="eyebrow">Agent Skills · 用户级</p>
        <h1>Skills</h1>
        <p>统一保存技能，分发到 Claude Code、Codex 和 Gemini CLI。</p>
      </div>
      <button
        class="button secondary"
        :disabled="store.loading"
        @click="store.load"
      >
        刷新列表
      </button>
    </header>
    <div v-if="store.error" class="inline-error" role="alert">
      {{ store.error }}
    </div>
    <nav class="extension-tabs" aria-label="Skill 管理视图">
      <button
        v-for="item in tabs"
        :key="item.id"
        :class="{ active: tab === item.id }"
        @click="changeTab(item.id)"
      >
        {{ item.label }}
      </button>
    </nav>
    <OperationPanel resource="skill" />
    <fieldset :disabled="locked" class="extension-fieldset">
      <div class="extension-panel extension-toolbar">
        <label
          >目标应用<select v-model="target">
            <option value="">全部应用</option>
            <option
              v-for="agent in store.targets"
              :key="agent.id"
              :value="agent.id"
            >
              {{ agent.name
              }}{{ agent.sharedWith.length ? "（共享 Skill 目录）" : "" }}
            </option>
          </select></label
        >
        <label
          >部署方式<select v-model="mode">
            <option value="auto">自动：优先链接</option>
            <option value="symlink">符号链接</option>
            <option value="copy">复制</option>
          </select></label
        >
        <small
          >启停会作用于共享目录的关联应用。安装时不选应用，可只保留在管理库。</small
        >
      </div>
      <template v-if="tab === 'installed'">
        <div class="extension-toolbar">
          <label class="search-field"
            ><span class="sr-only">搜索已安装 Skill</span
            ><input
              v-model="query"
              type="search"
              placeholder="搜索名称、描述或来源" /></label
          ><label
            >状态<select v-model="enabledFilter">
              <option value="">全部状态</option>
              <option value="enabled">已启用</option>
              <option value="disabled">全部停用</option>
              <option value="external">外部安装</option>
            </select></label
          ><label
            >来源<select v-model="sourceFilter">
              <option value="">全部来源</option>
              <option v-for="source in sourceUris" :key="source">
                {{ source }}
              </option>
            </select></label
          >
        </div>
        <div class="extension-toolbar">
          <label
            ><input
              type="checkbox"
              :checked="
                visible.length > 0 && selectedVisible.length === visible.length
              "
              @change="selectVisible"
            />
            当前筛选 {{ visible.length }} 项</label
          ><span>已选 {{ selectedVisible.length }} 项</span
          ><button
            class="button secondary"
            :disabled="!selectedVisible.length || !target"
            @click="batch('toggle', true)"
          >
            启用选中项</button
          ><button
            class="button secondary"
            :disabled="!selectedVisible.length || !target"
            @click="batch('toggle', false)"
          >
            停用选中项</button
          ><button
            class="button secondary"
            :disabled="!selectedVisible.length"
            @click="store.check(selectedVisible)"
          >
            检查选中项更新</button
          ><button
            class="button danger"
            :disabled="!selectedVisible.length"
            @click="batch('uninstall')"
          >
            卸载选中项
          </button>
        </div>
        <p v-if="store.loading" role="status">正在读取…</p>
        <AppEmptyState
          v-else-if="!visible.length"
          title="暂无匹配的 Skill"
          description="从「发现」安装，或扫描应用中的已有安装。全部停用后，内容仍保存在管理库。"
          mark="✦"
        />
        <article
          v-for="skill in visible"
          :key="skill.id"
          class="extension-panel resource-card"
        >
          <header class="extension-heading">
            <label class="resource-title"
              ><input
                v-model="selection"
                type="checkbox"
                :value="skill.id"
                :aria-label="`选择 ${skill.name}`"
              />
              <h2>{{ skill.name }}</h2></label
            ><span class="badge">{{
              skill.pendingDelete ? "待清理" : "已保存到库"
            }}</span>
          </header>
          <p>{{ skill.description }}</p>
          <p class="muted">
            {{ skill.origin?.source.uri ?? "外部导入 · 来源未知"
            }}<template v-if="skill.origin">
              · {{ skill.origin.relativePath || "/" }} ·
              {{ skill.origin.requestedRef ?? "默认分支" }}</template
            >
          </p>
          <small
            >更新时间 {{ new Date(skill.updatedAt).toLocaleString() }} · ID
            {{ skill.id.slice(0, 8) }}</small
          >
          <div class="agent-bindings">
            <div
              v-for="agent in store.targets"
              :key="agent.id"
              class="agent-binding"
            >
              <strong>{{ agent.name }}</strong
              ><span>{{ deploymentLabel(skill, agent.id) }}</span
              ><button
                v-if="deployment(skill, agent.id)?.mode === 'external'"
                class="button secondary compact"
                @click="
                  store.preview('adopt', {
                    ids: [skill.id],
                    targets: [agent.id],
                    mode,
                  })
                "
              >
                预览接管</button
              ><button
                v-else
                class="button secondary compact"
                @click="
                  store.preview('toggle', {
                    ids: [skill.id],
                    targets: [agent.id],
                    enabled: !deployment(skill, agent.id)?.desiredEnabled,
                    mode,
                  })
                "
              >
                {{
                  deployment(skill, agent.id)?.desiredEnabled ? "停用" : "启用"
                }}
              </button>
            </div>
          </div>
          <p v-if="updateFor(skill.id)" class="inline-warning">
            {{ updateFor(skill.id)?.message }}
          </p>
          <div class="extension-actions">
            <button class="text-button" @click="store.check([skill.id])">
              只检查更新</button
            ><button
              v-if="updateFor(skill.id)?.status === 'update_available'"
              class="button primary compact"
              @click="updateSkill(skill.id)"
            >
              预览更新</button
            ><button
              class="text-button"
              @click="store.preview('uninstall', { ids: [skill.id] })"
            >
              卸载并备份
            </button>
          </div>
        </article>
        <button
          v-if="availableUpdates.length"
          class="button primary"
          @click="updateAll"
        >
          更新当前筛选内可更新的 {{ availableUpdates.length }} 项
        </button>
      </template>
      <template v-else-if="tab === 'discover'">
        <form
          class="extension-panel"
          @submit.prevent="store.discover(sourceDraft)"
        >
          <h2>发现来源中的 Skill</h2>
          <div class="field-grid">
            <label
              >来源类型<select v-model="sourceDraft.kind">
                <option value="github">公开 GitHub</option>
                <option value="local">本地目录</option>
                <option value="zip">本地 ZIP</option>
              </select></label
            ><label
              >仓库或绝对路径<input
                v-model="sourceDraft.uri"
                required
                :placeholder="
                  sourceDraft.kind === 'github'
                    ? 'owner/repository'
                    : '/absolute/path'
                " /></label
            ><label v-if="sourceDraft.kind === 'github'"
              >分支 / tag / commit<input
                v-model="sourceDraft.requestedRef"
                placeholder="默认分支"
            /></label>
          </div>
          <button class="button primary" :disabled="store.discovering">
            发现 Skill
          </button>
        </form>
        <form
          class="extension-panel extension-toolbar"
          @submit.prevent="store.search(remoteQuery)"
        >
          <label
            >skills.sh 搜索<input
              v-model="remoteQuery"
              required
              placeholder="搜索公开 Skill" /></label
          ><button class="button secondary" :disabled="store.discovering">
            搜索
          </button>
        </form>
        <div
          v-for="(entry, index) in store.searchResults"
          :key="index"
          class="extension-panel extension-toolbar"
        >
          <strong>{{ entry.name }}</strong
          ><span>{{ entry.repository }}</span
          ><button class="text-button" @click="discoverRepo(entry.repository)">
            发现此仓库
          </button>
        </div>
        <div class="extension-toolbar">
          <label v-for="agent in store.targets" :key="agent.id"
            ><input
              v-model="installTargets"
              type="checkbox"
              :value="agent.id"
            />
            {{ agent.name }}</label
          ><span v-if="!installTargets.length">仅保存到管理库</span>
        </div>
        <article
          v-for="candidate in store.candidates"
          :key="candidate.id"
          class="extension-panel"
        >
          <header class="extension-heading">
            <h2>{{ candidate.name }}</h2>
            <button
              class="button primary compact"
              @click="
                store.preview('install', {
                  candidates: [candidate.id],
                  targets: installTargets,
                  mode,
                })
              "
            >
              预览安装
            </button>
          </header>
          <p>{{ candidate.description }}</p>
          <code>{{ candidate.origin?.relativePath || "/" }}</code>
          <details>
            <summary>查看 SKILL.md</summary>
            <pre class="skill-content">{{ candidate.content }}</pre>
          </details>
        </article>
      </template>
      <template v-else-if="tab === 'imports'">
        <div class="extension-panel">
          <h2>导入已有安装</h2>
          <p>
            每个实际路径分别展示。导入保存快照，原目录保持不变；导入后可显式接管应用部署。
          </p>
          <button
            class="button secondary"
            :disabled="store.discovering"
            @click="store.scanImports"
          >
            扫描已有 Skill
          </button>
        </div>
        <article
          v-for="candidate in store.imports"
          :key="candidate.id"
          class="extension-panel"
        >
          <header class="extension-heading">
            <h2>{{ candidate.name }}</h2>
            <button
              class="button primary compact"
              @click="store.preview('import', { candidates: [candidate.id] })"
            >
              导入此快照
            </button>
          </header>
          <p>{{ candidate.targetIds.join("、") }}</p>
          <code>{{ candidate.path }}</code>
          <p v-if="candidate.linkTarget">
            链接目标：{{ candidate.linkTarget }}
          </p>
          <p>内容指纹：{{ candidate.contentHash.slice(0, 16) }}</p>
        </article>
        <p v-if="!store.imports.length">尚无候选；点击扫描读取应用目录。</p>
      </template>
      <template v-else-if="tab === 'sources'">
        <form
          class="extension-panel"
          @submit.prevent="store.saveSource(sourceDraft)"
        >
          <h2>添加仓库或本地来源</h2>
          <div class="field-grid">
            <label
              >类型<select v-model="sourceDraft.kind">
                <option value="github">GitHub</option>
                <option value="local">本地目录</option>
                <option value="zip">本地 ZIP</option>
              </select></label
            ><label
              >仓库 / 路径<input v-model="sourceDraft.uri" required /></label
            ><label>ref<input v-model="sourceDraft.requestedRef" /></label>
          </div>
          <div class="extension-actions">
            <button class="button primary">
              {{ sourceDraft.id ? "保存来源修改" : "保存来源" }}</button
            ><button
              type="button"
              class="text-button"
              @click="
                Object.assign(sourceDraft, {
                  id: '',
                  kind: 'github',
                  uri: '',
                  requestedRef: null,
                  enabled: true,
                })
              "
            >
              新建来源
            </button>
          </div>
        </form>
        <article
          v-for="source in store.snapshot.sources"
          :key="source.id"
          class="extension-panel"
        >
          <h2>{{ source.uri }}</h2>
          <p>
            {{ source.kind }} · {{ source.requestedRef || "默认分支" }} ·
            {{ source.enabled ? "启用" : "停用" }}
          </p>
          <div class="extension-actions">
            <button
              class="button secondary"
              :disabled="!source.enabled"
              @click="
                tab = 'discover';
                store.discover(source);
              "
            >
              发现</button
            ><button
              class="text-button"
              @click="store.saveSource({ ...source, enabled: !source.enabled })"
            >
              {{ source.enabled ? "停用来源" : "启用来源" }}</button
            ><button
              class="text-button"
              @click="Object.assign(sourceDraft, source)"
            >
              编辑来源</button
            ><button class="text-button" @click="store.deleteSource(source.id)">
              删除来源
            </button>
          </div>
        </article>
      </template>
      <template v-else>
        <p>
          备份保留原内容、来源和应用状态；恢复也会检查当前文件是否发生变化。
        </p>
        <article
          v-for="backup in store.backups"
          :key="backup.id"
          class="extension-panel"
        >
          <h2>{{ backup.names.join("、") }}</h2>
          <p>
            {{ backup.operation }} · {{ backup.status }} ·
            {{ new Date(backup.createdAt).toLocaleString() }} ·
            {{ Math.ceil(Number(backup.sizeBytes) / 1024) }} KiB
          </p>
          <div class="extension-actions">
            <button
              class="button secondary"
              @click="store.preview('restore', { backupId: backup.id })"
            >
              预览恢复</button
            ><button
              class="text-button"
              :disabled="!['succeeded', 'restored'].includes(backup.status)"
              @click="store.preview('delete_backup', { backupId: backup.id })"
            >
              删除备份
            </button>
          </div>
        </article>
        <p v-if="!store.backups.length">暂无备份。</p>
      </template>
    </fieldset>
    <div v-if="store.discovering" class="inline-warning" role="status">
      正在读取来源…<button class="text-button" @click="store.cancelRequest">
        取消请求
      </button>
    </div>
    <p
      v-for="(diagnostic, index) in store.diagnostics"
      :key="index"
      class="inline-error"
      role="alert"
    >
      {{ diagnostic.targetId }}：{{ diagnostic.message }}
    </p>
    <AgentPaths :targets="store.targets" @changed="store.load" />
  </div>
</template>
<script setup lang="ts">
import {
  computed,
  onBeforeUnmount,
  onMounted,
  reactive,
  ref,
  watch,
} from "vue";
import type { DeployMode, SkillRecord, SkillSource } from "../bindings";
import { useSkillsStore } from "../stores/skills";
import { useExtensionOperations } from "../stores/extensions";
import AppEmptyState from "../components/AppEmptyState.vue";
import OperationPanel from "../components/operations/OperationPanel.vue";
import AgentPaths from "../components/operations/AgentPaths.vue";
const store = useSkillsStore(),
  operations = useExtensionOperations();
const tabs = [
  { id: "installed", label: "已安装" },
  { id: "discover", label: "发现" },
  { id: "sources", label: "来源" },
  { id: "imports", label: "导入已有安装" },
  { id: "backups", label: "备份恢复" },
];
const tab = ref("installed"),
  target = ref(""),
  query = ref(""),
  enabledFilter = ref(""),
  sourceFilter = ref(""),
  remoteQuery = ref("");
const mode = ref<DeployMode>("auto"),
  selection = ref<string[]>([]),
  installTargets = ref<string[]>([]);
const sourceDraft = reactive<SkillSource>({
  id: "",
  kind: "github",
  uri: "",
  requestedRef: null,
  enabled: true,
});
const locked = computed(() => operations.pending || operations.preparing);
const sourceUris = computed(() => [
  ...new Set(
    store.snapshot.skills
      .map((s) => s.origin?.source.uri)
      .filter((s): s is string => Boolean(s)),
  ),
]);
const visible = computed(() =>
  store.snapshot.skills.filter(
    (s) =>
      (!target.value ||
        s.deployments.some((d) => d.targetId === target.value)) &&
      (!sourceFilter.value || s.origin?.source.uri === sourceFilter.value) &&
      (!query.value ||
        `${s.name} ${s.description} ${s.origin?.source.uri ?? ""}`
          .toLowerCase()
          .includes(query.value.toLowerCase())) &&
      (!enabledFilter.value ||
        (enabledFilter.value === "external"
          ? s.deployments.some((d) => d.mode === "external")
          : enabledFilter.value === "enabled"
            ? s.deployments.some((d) => d.desiredEnabled)
            : s.deployments.every((d) => !d.desiredEnabled))),
  ),
);
const selectedVisible = computed(() =>
  visible.value.filter((s) => selection.value.includes(s.id)).map((s) => s.id),
);
const availableUpdates = computed(() =>
  store.updates.filter(
    (u) =>
      u.status === "update_available" &&
      u.candidateId &&
      visible.value.some((s) => s.id === u.skillId),
  ),
);
function deployment(s: SkillRecord, id: string) {
  return s.deployments.find((d) => d.targetId === id);
}
function deploymentLabel(s: SkillRecord, id: string) {
  const d = deployment(s, id);
  return d
    ? `${d.observedState} · ${d.mode}${d.desiredEnabled && d.observedState === "disabled" ? " · 等待同步" : ""}`
    : "未启用";
}
function updateFor(id: string) {
  return store.updates.find((u) => u.skillId === id);
}
function selectVisible(event: Event) {
  selection.value = (event.target as HTMLInputElement).checked
    ? visible.value.map((s) => s.id)
    : [];
}
function batch(action: "toggle" | "uninstall", enabled = true) {
  void store.preview(action, {
    ids: selectedVisible.value,
    targets: target.value ? [target.value] : [],
    enabled,
    mode: mode.value,
  });
}
function updateSkill(id: string) {
  const update = updateFor(id);
  if (update?.candidateId)
    void store.preview("update", {
      ids: [id],
      candidates: [update.candidateId],
      mode: mode.value,
    });
}
function updateAll() {
  void store.preview("update", {
    ids: availableUpdates.value.map((u) => u.skillId),
    candidates: availableUpdates.value.map((u) => u.candidateId!),
    mode: mode.value,
  });
}
function discoverRepo(uri: string) {
  sourceDraft.kind = "github";
  sourceDraft.uri = uri;
  void store.discover({ ...sourceDraft });
}
function changeTab(id: string) {
  tab.value = id;
}
watch(
  () => operations.pending,
  (pending, was) => {
    if (was && !pending) {
      void store.load();
      store.imports = [];
    }
  },
);
onMounted(() => {
  void store.load();
  void store.connectEvents().catch(() => undefined);
});
onBeforeUnmount(() => store.disconnectEvents());
</script>
