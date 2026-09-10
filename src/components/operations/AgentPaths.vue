<template>
  <details class="extension-panel">
    <summary>应用路径与支持范围 · 用户级</summary>
    <p>Codex / Gemini 默认共享 Skill 目录。缺失目录仅在确认写入时初始化。</p>
    <div v-for="target in targets" :key="target.id" class="agent-path">
      <strong
        >{{ target.name }} ·
        {{ target.available ? "已初始化" : "尚未初始化" }} ·
        {{ target.pathSource }}</strong
      >
      <p>
        Skill：<code>{{ target.skillsDir }}</code>
      </p>
      <p>
        MCP：<code>{{ target.mcpFile }}</code>
      </p>
      <small
        >支持 {{ target.transports.join(" / ")
        }}<template v-if="target.sharedWith.length">
          · Skill 关联 {{ target.sharedWith.join("、") }}</template
        ></small
      >
    </div>
    <details>
      <summary>覆盖路径</summary>
      <p>已有部署绑定时禁止切换路径；请先移除绑定。留空使用默认/环境路径。</p>
      <form @submit.prevent="save">
        <div v-for="target in targets" :key="target.id" class="field-grid">
          <label
            >{{ target.name }} Skill 目录<input
              v-model="draft[target.id]!.skillsDir"
              placeholder="绝对目录路径"
          /></label>
          <label
            >{{ target.name }} MCP 文件<input
              v-model="draft[target.id]!.mcpFile"
              placeholder="绝对文件路径"
          /></label>
        </div>
        <p v-if="message" role="status">{{ message }}</p>
        <button class="button secondary" :disabled="saving">
          保存路径覆盖
        </button>
      </form>
    </details>
  </details>
</template>
<script setup lang="ts">
import { reactive, ref, watch } from "vue";
import type { AgentTarget } from "../../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../../api/transport";
const props = defineProps<{ targets: AgentTarget[] }>();
const emit = defineEmits<{ changed: [] }>();
const draft = reactive<Record<string, { skillsDir: string; mcpFile: string }>>(
  {},
);
const saving = ref(false),
  message = ref("");
watch(
  () => props.targets,
  (list) => {
    for (const t of list)
      draft[t.id] = {
        skillsDir: t.pathSource === "override" ? t.skillsDir : "",
        mcpFile: t.pathSource === "override" ? t.mcpFile : "",
      };
  },
  { immediate: true },
);
async function save() {
  saving.value = true;
  try {
    const current = await unwrapCommand(getTransport().commands.listSkills());
    await unwrapCommand(
      getTransport().commands.saveAgentTargets({
        expectedRevision: current.revision,
        overrides: Object.fromEntries(
          Object.entries(draft).map(([id, d]) => [
            id,
            { skillsDir: d.skillsDir || null, mcpFile: d.mcpFile || null },
          ]),
        ),
      }),
    );
    message.value = "路径已保存";
    emit("changed");
  } catch (e) {
    message.value = errorMessage(e, "保存失败");
  } finally {
    saving.value = false;
  }
}
</script>
