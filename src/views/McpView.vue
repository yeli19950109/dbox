<template>
  <div class="view extension-view">
    <header class="extension-heading">
      <div>
        <p class="eyebrow">Model Context Protocol · 用户级</p>
        <h1>MCP 服务器</h1>
        <p>保存服务器配置，按应用启停与同步。</p>
      </div>
      <div class="extension-actions">
        <button class="button secondary" @click="store.load">刷新</button
        ><button
          class="button primary"
          :disabled="locked || store.loading"
          @click="openEditor(null)"
        >
          添加服务器
        </button>
      </div>
    </header>
    <p v-if="store.error" class="inline-error" role="alert">
      {{ store.error }}
    </p>
    <nav class="extension-tabs" aria-label="MCP 管理视图">
      <button :class="{ active: tab === 'servers' }" @click="tab = 'servers'">
        服务器</button
      ><button :class="{ active: tab === 'imports' }" @click="tab = 'imports'">
        导入</button
      ><button :class="{ active: tab === 'backups' }" @click="tab = 'backups'">
        备份恢复
      </button>
    </nav>
    <OperationPanel resource="mcp" />
    <McpEditor
      v-if="editing"
      :key="editKey"
      :server="edited"
      :targets="store.targets"
      :disabled="locked"
      @close="editing = false"
      @save="save"
    />
    <fieldset class="extension-fieldset" :disabled="locked">
      <template v-if="tab === 'servers'">
        <div class="extension-toolbar">
          <label class="search-field"
            ><span class="sr-only">搜索 MCP 服务器</span
            ><input
              v-model="query"
              type="search"
              placeholder="搜索名称或描述（不搜索配置与密钥）" /></label
          ><label
            >应用<select v-model="target">
              <option value="">全部应用</option>
              <option
                v-for="agent in store.targets"
                :key="agent.id"
                :value="agent.id"
              >
                {{ agent.name }}
              </option>
            </select></label
          ><label
            >同步状态<select v-model="stateFilter">
              <option value="">全部</option>
              <option value="error">需要处理</option>
              <option value="enabled">期望启用</option>
              <option value="disabled">全部停用</option>
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
            @click="batchToggle(true)"
          >
            启用选中项</button
          ><button
            class="button secondary"
            :disabled="!selectedVisible.length || !target"
            @click="batchToggle(false)"
          >
            停用选中项</button
          ><button
            class="button secondary"
            :disabled="!selectedVisible.length"
            @click="
              store.preview('sync', {
                ids: selectedVisible,
                targets: target ? [target] : [],
              })
            "
          >
            同步选中项</button
          ><button
            class="button danger"
            :disabled="!selectedVisible.length"
            @click="store.preview('delete', { ids: selectedVisible })"
          >
            删除选中项
          </button>
        </div>
        <p v-if="store.loading" role="status">正在读取 MCP 配置…</p>
        <AppEmptyState
          v-else-if="!visible.length"
          title="暂无匹配的服务器"
          description="添加配置，或导入应用中已有的 MCP 服务器。"
          mark="⌘"
        />
        <article
          v-for="server in visible"
          :key="server.id"
          class="extension-panel resource-card"
        >
          <header class="extension-heading">
            <label class="resource-title"
              ><input
                v-model="selection"
                type="checkbox"
                :value="server.id"
                :aria-label="`选择 ${server.name}`"
              />
              <h2>{{ server.name }}</h2></label
            ><span class="badge"
              >{{ server.transport
              }}{{ server.pendingDelete ? " · 待清理" : "" }}</span
            >
          </header>
          <p>{{ server.description }}</p>
          <small>ID {{ server.id.slice(0, 8) }}</small>
          <div class="agent-bindings">
            <div
              v-for="agent in store.targets"
              :key="agent.id"
              class="agent-binding"
            >
              <strong>{{ agent.name }}</strong
              ><span
                >期望：{{
                  binding(server, agent.id)?.desiredEnabled ? "启用" : "停用"
                }}
                · 实际：{{
                  binding(server, agent.id)?.observedState ?? "未配置"
                }}</span
              ><code v-if="binding(server, agent.id)">{{
                binding(server, agent.id)?.key
              }}</code
              ><button
                class="button secondary compact"
                @click="
                  store.preview('toggle', {
                    ids: [server.id],
                    targets: [agent.id],
                    enabled: !binding(server, agent.id)?.desiredEnabled,
                  })
                "
              >
                {{
                  binding(server, agent.id)?.desiredEnabled ? "停用" : "启用"
                }}</button
              ><button
                v-if="needsSync(server, agent.id)"
                class="text-button"
                @click="
                  store.preview('sync', {
                    ids: [server.id],
                    targets: [agent.id],
                  })
                "
              >
                重试此应用
              </button>
            </div>
          </div>
          <details>
            <summary>查看脱敏配置</summary>
            <pre>{{ server.configJson }}</pre>
          </details>
          <div class="extension-actions">
            <button class="text-button" @click="openEditor(server)">编辑</button
            ><button
              class="text-button"
              @click="store.preview('sync', { ids: [server.id] })"
            >
              手动同步</button
            ><button
              class="text-button"
              @click="store.preview('delete', { ids: [server.id] })"
            >
              删除并备份
            </button>
          </div>
        </article>
      </template>
      <template v-else-if="tab === 'imports'">
        <section class="extension-panel">
          <h2>从应用配置导入</h2>
          <p>
            同名条目按应用和配置 key 分别保留。导入只保存 dbox
            记录，不改写应用配置。
          </p>
          <button class="button secondary" @click="store.scanImports">
            扫描应用
          </button>
          <div
            v-for="(diagnostic, index) in store.scan.errors"
            :key="index"
            class="inline-error"
            role="alert"
          >
            {{ diagnostic.targetId }}：{{ diagnostic.message }}
          </div>
        </section>
        <article
          v-for="candidate in store.scan.candidates"
          :key="candidate.id"
          class="extension-panel"
        >
          <header class="extension-heading">
            <h2>{{ candidate.key }} · {{ candidate.targetId }}</h2>
            <button
              class="button primary compact"
              @click="store.preview('import', { candidates: [candidate.id] })"
            >
              导入此条目
            </button>
          </header>
          <p>
            {{ candidate.transport }} ·
            {{ candidate.enabled ? "已启用" : "已停用" }}
          </p>
          <pre>{{ candidate.configJson }}</pre>
        </article>
        <form
          class="extension-panel"
          @submit.prevent="store.preview('import', { importJson })"
        >
          <h2>JSON 导入</h2>
          <label
            >mcpServers 对象<textarea
              v-model="importJson"
              rows="8"
              spellcheck="false"
              placeholder='{"mcpServers":{"example":{"command":"npx","args":[]}}}'
              required
            /></label
          ><button class="button primary">预览导入</button>
        </form>
      </template>
      <template v-else
        ><p>恢复会校验原操作之后的修改；存在冲突时保留当前文件。</p>
        <article
          v-for="backup in store.backups"
          :key="backup.id"
          class="extension-panel"
        >
          <h2>{{ backup.names.join("、") }}</h2>
          <p>
            {{ backup.operation }} · {{ backup.status }} ·
            {{ new Date(backup.createdAt).toLocaleString() }}
          </p>
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
        </article>
        <p v-if="!store.backups.length">暂无备份。</p></template
      >
    </fieldset>
    <AgentPaths :targets="store.targets" @changed="store.load" />
  </div>
</template>
<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { McpEdit, McpServerView } from "../bindings";
import { useMcpStore } from "../stores/mcp";
import { useExtensionOperations } from "../stores/extensions";
import OperationPanel from "../components/operations/OperationPanel.vue";
import AgentPaths from "../components/operations/AgentPaths.vue";
import McpEditor from "../components/mcp/McpEditor.vue";
import AppEmptyState from "../components/AppEmptyState.vue";
const store = useMcpStore(),
  operations = useExtensionOperations();
const query = ref(""),
  target = ref(""),
  stateFilter = ref(""),
  tab = ref("servers"),
  importJson = ref(""),
  selection = ref<string[]>([]);
const editing = ref(false),
  edited = ref<McpServerView | null>(null),
  editKey = ref(0);
const locked = computed(() => operations.pending || operations.preparing);
function binding(server: McpServerView, id: string) {
  return server.bindings.find((b) => b.targetId === id);
}
function needsSync(server: McpServerView, id: string) {
  const b = binding(server, id);
  return b && b.observedState !== (b.desiredEnabled ? "enabled" : "disabled");
}
const visible = computed(() =>
  store.snapshot.servers.filter(
    (s) =>
      (!query.value ||
        `${s.name} ${s.description}`
          .toLowerCase()
          .includes(query.value.toLowerCase())) &&
      (!target.value || s.bindings.some((b) => b.targetId === target.value)) &&
      (!stateFilter.value ||
        (stateFilter.value === "enabled"
          ? s.bindings.some((b) => b.desiredEnabled)
          : stateFilter.value === "disabled"
            ? s.bindings.every((b) => !b.desiredEnabled)
            : s.bindings.some((b) => needsSync(s, b.targetId)))),
  ),
);
const selectedVisible = computed(() =>
  visible.value.filter((s) => selection.value.includes(s.id)).map((s) => s.id),
);
function selectVisible(event: Event) {
  selection.value = (event.target as HTMLInputElement).checked
    ? visible.value.map((s) => s.id)
    : [];
}
function batchToggle(enabled: boolean) {
  void store.preview("toggle", {
    ids: selectedVisible.value,
    targets: [target.value],
    enabled,
  });
}
function openEditor(server: McpServerView | null) {
  edited.value = server;
  editKey.value++;
  editing.value = true;
}
async function save(edit: McpEdit) {
  await store.preview("upsert", { edit });
  if (operations.plan) editing.value = false;
}
watch(
  () => operations.pending,
  (pending, was) => {
    if (was && !pending) {
      void store.load();
      store.scan = { candidates: [], errors: [] };
      importJson.value = "";
    }
  },
);
onMounted(() => {
  void store.load();
  void store.connectEvents().catch(() => undefined);
});
onBeforeUnmount(() => store.disconnectEvents());
</script>
