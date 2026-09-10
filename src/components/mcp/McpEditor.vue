<template>
  <form
    class="extension-panel"
    aria-label="MCP 配置编辑"
    @submit.prevent="save"
  >
    <header class="extension-heading">
      <h2>{{ server ? "编辑服务器" : "添加服务器" }}</h2>
      <button type="button" class="text-button" @click="$emit('close')">
        关闭
      </button>
    </header>
    <div class="field-grid">
      <label>显示名称<input v-model="draft.name" required /></label
      ><label>描述<input v-model="draft.description" /></label
      ><label
        >Transport<select v-model="draft.transport" @change="changeTransport">
          <option value="stdio">stdio</option>
          <option value="http">HTTP</option>
          <option value="sse">SSE</option>
        </select></label
      >
    </div>
    <p>配置仅用于应用连接。dbox 不会启动 MCP 进程。</p>
    <div v-if="draft.transport === 'stdio'" class="field-grid">
      <label
        >可执行文件<input
          :value="field('command')"
          placeholder="npx 或绝对路径"
          @input="
            setField('command', ($event.target as HTMLInputElement).value)
          " /></label
      ><label
        >参数（JSON 数组）<input
          :value="JSON.stringify(config.args ?? [])"
          placeholder='["-y", "package-name"]'
          @change="setArgs(($event.target as HTMLInputElement).value)"
      /></label>
    </div>
    <label v-else
      >服务 URL<input
        :value="field('url')"
        placeholder="https://example.com/mcp"
        @input="setField('url', ($event.target as HTMLInputElement).value)"
    /></label>
    <details :open="!server">
      <summary>配置 JSON（env / headers / cwd）</summary>
      <textarea
        v-model="draft.configJson"
        class="config-editor"
        rows="9"
        spellcheck="false"
        aria-label="MCP 配置 JSON"
      />
      <p>
        env、headers 和 URL 等已有值显示为
        [REDACTED]；不改动即保留。应用专有字段放在对应应用下。
      </p>
    </details>
    <div v-if="secretPointers.length" class="secret-edits">
      <h3>已有敏感字段</h3>
      <div v-for="pointer in secretPointers" :key="pointer" class="field-grid">
        <label
          >{{ pointer
          }}<select
            :value="draft.secrets[pointer]?.action ?? 'keep'"
            @change="
              setSecretAction(
                pointer,
                ($event.target as HTMLSelectElement).value,
              )
            "
          >
            <option value="keep">保留原值</option>
            <option value="set">设置新值</option>
            <option v-if="!/^\/args\//.test(pointer)" value="remove">
              移除字段
            </option>
          </select></label
        ><label v-if="draft.secrets[pointer]?.action === 'set'"
          >新值<input
            type="password"
            autocomplete="off"
            :value="secretValue(pointer)"
            @input="
              draft.secrets[pointer] = {
                action: 'set',
                value: ($event.target as HTMLInputElement).value,
              }
            "
        /></label>
      </div>
    </div>
    <h3>目标应用</h3>
    <div
      v-for="binding in draft.bindings"
      :key="binding.targetId"
      class="mcp-editor-binding"
    >
      <div class="extension-toolbar">
        <label
          ><input v-model="binding.desiredEnabled" type="checkbox" />
          {{ targetName(binding.targetId) }}</label
        ><span v-if="!supports(binding.targetId)" class="inline-warning"
          >不支持当前 transport；预览会标出此目标</span
        ><label
          >配置 key<input
            v-model="binding.key"
            :placeholder="draft.name || '与显示名称相同'"
        /></label>
      </div>
      <details>
        <summary>此应用的高级字段</summary>
        <textarea
          v-model="binding.extraJson"
          rows="3"
          spellcheck="false"
          :aria-label="`${targetName(binding.targetId)} 高级字段`"
        />
      </details>
    </div>
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>
    <div class="extension-actions">
      <button type="button" class="button secondary" @click="validate">
        校验配置</button
      ><button class="button primary" :disabled="disabled">预览保存</button>
    </div>
  </form>
</template>
<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import type {
  AgentTarget,
  McpServerView,
  McpEdit,
  SecretEdit,
} from "../../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../../api/transport";
const props = defineProps<{
  server: McpServerView | null;
  targets: AgentTarget[];
  disabled: boolean;
}>();
const emit = defineEmits<{ save: [edit: McpEdit]; close: [] }>();
const draft = reactive<McpEdit>({
  id: props.server?.id ?? null,
  name: props.server?.name ?? "",
  description: props.server?.description ?? "",
  transport: props.server?.transport ?? "stdio",
  configJson:
    props.server?.configJson ??
    JSON.stringify({ command: "npx", args: [], env: {} }, null, 2),
  secrets: {},
  bindings: props.targets.map((t) => {
    const b = props.server?.bindings.find((b) => b.targetId === t.id);
    return b
      ? { ...b }
      : {
          targetId: t.id,
          key: props.server?.name ?? "",
          desiredEnabled: false,
          observedState: "disabled",
          extraJson: "{}",
        };
  }),
});
const error = ref("");
const config = computed<Record<string, unknown>>(() => {
  try {
    return JSON.parse(draft.configJson);
  } catch {
    return {};
  }
});
function field(key: string) {
  return typeof config.value[key] === "string"
    ? (config.value[key] as string)
    : "";
}
function setField(key: string, value: string) {
  draft.configJson = JSON.stringify({ ...config.value, [key]: value }, null, 2);
}
function setArgs(text: string) {
  try {
    draft.configJson = JSON.stringify(
      { ...config.value, args: JSON.parse(text) },
      null,
      2,
    );
    error.value = "";
  } catch {
    error.value = "参数必须为 JSON 字符串数组";
  }
}
function changeTransport() {
  draft.configJson = JSON.stringify(
    draft.transport === "stdio"
      ? { command: "npx", args: [], env: {} }
      : { url: "", headers: {} },
    null,
    2,
  );
  draft.secrets = {};
}
function pointers(value: unknown, path = ""): string[] {
  if (value === "[REDACTED]") return [path];
  if (typeof value !== "object" || value === null) return [];
  return Object.entries(value).flatMap(([key, child]) =>
    pointers(child, `${path}/${key.replace(/~/g, "~0").replace(/\//g, "~1")}`),
  );
}
const secretPointers = computed(() => pointers(config.value));
function setSecretAction(pointer: string, action: string) {
  draft.secrets[pointer] =
    action === "set"
      ? { action: "set", value: "" }
      : ({ action } as SecretEdit);
}
function secretValue(pointer: string) {
  const e = draft.secrets[pointer];
  return e?.action === "set" ? e.value : "";
}
function targetName(id: string) {
  return props.targets.find((t) => t.id === id)?.name ?? id;
}
function supports(id: string) {
  return props.targets
    .find((t) => t.id === id)
    ?.transports.includes(draft.transport);
}
function payload(): McpEdit {
  return {
    ...draft,
    bindings: draft.bindings.map((b) => ({ ...b, key: b.key || draft.name })),
  };
}
async function validate() {
  try {
    const diagnostics = await unwrapCommand(
      getTransport().commands.validateMcpServer(payload()),
    );
    error.value = diagnostics.length
      ? diagnostics.map((d) => `${d.targetId}：${d.message}`).join("；")
      : "配置校验通过";
  } catch (e) {
    error.value = errorMessage(e, "配置无效");
  }
}
function save() {
  emit("save", payload());
}
</script>
