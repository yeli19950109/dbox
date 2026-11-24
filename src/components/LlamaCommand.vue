<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useLocalStorage } from "@vueuse/core";
import type { LlamaInstalledVersion } from "../bindings";
import { errorMessage, getTransport, unwrapCommand } from "../api/transport";

const props = defineProps<{ version?: LlamaInstalledVersion }>();
const config = useLocalStorage("dbox.llama.command", {
  model: "", host: "127.0.0.1", port: "8080", context: "", threads: "", gpuLayers: "",
}, { mergeDefaults: true });
const apiKey = ref("");
const reveal = ref(false);
const choosing = ref(false);
const pickerError = ref("");
const copyStatus = ref("");

const numericOptions = [
  { key: "context", flag: "--ctx-size", label: "上下文长度", min: 0 },
  { key: "threads", flag: "--threads", label: "CPU 线程数", min: 1 },
  { key: "gpuLayers", flag: "--n-gpu-layers", label: "GPU 层数", min: 0 },
] as const;

function integerInRange(value: string, min: number, max: number) {
  return /^\d+$/.test(value) && Number(value) >= min && Number(value) <= max;
}

const validation = computed(() => {
  if (!config.value.host.trim()) return "请输入监听地址。";
  if (!integerInRange(config.value.port, 1, 65535)) return "端口必须是 1 到 65535 之间的整数。";
  for (const option of numericOptions) {
    const value = config.value[option.key];
    if (value && !integerInRange(value, option.min, 2147483647)) {
      return `${option.label}请输入 ${option.min} 到 2147483647 之间的整数，或留空以使用版本默认值。`;
    }
  }
  return "";
});

function shellWord(value: string) {
  if (/^[a-zA-Z0-9_./:-]+$/.test(value)) return value;
  return "'" + value.replaceAll("'", "'\\''") + "'";
}

function command(key: string) {
  if (!props.version || !config.value.model || validation.value) return "";
  const args = [
    props.version.serverPath, "-m", config.value.model,
    "--host", config.value.host.trim(), "--port", config.value.port,
  ];
  if (key) args.push("--api-key", key);
  for (const { key, flag } of numericOptions) {
    if (config.value[key]) args.push(flag, config.value[key]);
  }
  return args.map(shellWord).join(" ");
}

const fullCommand = computed(() => command(apiKey.value));
const preview = computed(() => command(apiKey.value && !reveal.value ? "••••••••" : apiKey.value));
watch(fullCommand, () => { copyStatus.value = ""; });

async function chooseModel() {
  if (choosing.value) return;
  choosing.value = true;
  pickerError.value = "";
  try {
    const path = await unwrapCommand(getTransport().commands.llamaChooseModel());
    if (path) config.value.model = path;
  } catch (reason) {
    pickerError.value = errorMessage(reason, "无法打开模型文件选择窗口。");
  } finally {
    choosing.value = false;
  }
}

async function copyCommand() {
  const text = fullCommand.value;
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    if (text === fullCommand.value) copyStatus.value = "已复制，可在终端粘贴执行。";
  } catch {
    copyStatus.value = apiKey.value && !reveal.value
      ? "无法写入剪贴板。请显示密钥，再选中并复制运行命令。"
      : "无法写入剪贴板。请选中并复制运行命令。";
  }
}
</script>

<template>
  <section class="extension-panel command-panel" aria-labelledby="llama-config-title">
    <header class="extension-heading">
      <h2 id="llama-config-title">启动配置</h2>
      <code v-if="version" class="selected-version">{{ version.tag }}</code>
    </header>
    <div class="model-field" role="group" aria-labelledby="model-field-label">
      <span id="model-field-label">GGUF 模型</span>
      <div class="model-picker">
        <span class="model-path" :title="config.model">{{ config.model || "尚未选择模型文件" }}</span>
        <button class="button secondary compact" :disabled="choosing" @click="chooseModel">
          {{ choosing ? "选择中…" : config.model ? "更换模型" : "选择模型" }}
        </button>
      </div>
    </div>
    <p v-if="pickerError" class="inline-error" role="alert">{{ pickerError }}</p>
    <div class="field-grid">
      <label>监听地址<input v-model="config.host" type="text" spellcheck="false" /></label>
      <label>端口<input v-model.trim="config.port" inputmode="numeric" type="text" /></label>
    </div>
    <label for="llama-api-key">API Key（可选）</label>
    <div class="key-field">
      <input id="llama-api-key" v-model="apiKey" :type="reveal ? 'text' : 'password'" autocomplete="off" spellcheck="false" placeholder="输入 API Key" />
      <button class="text-button" :aria-pressed="reveal" @click="reveal = !reveal">{{ reveal ? "隐藏密钥" : "显示密钥" }}</button>
    </div>
    <details>
      <summary>更多参数</summary>
      <div class="field-grid three advanced-fields">
        <div v-for="option in numericOptions" :key="option.key" class="advanced-field">
          <div>
            <label :for="'llama-' + option.key">{{ option.label }}</label>
            <button v-if="option.key === 'gpuLayers'" class="text-button" title="最多卸载 999 层，兼容旧版 llama.cpp" @click="config.gpuLayers = '999'">全量（999）</button>
          </div>
          <input :id="'llama-' + option.key" v-model.trim="config[option.key]" type="text" inputmode="numeric" placeholder="版本默认值" />
        </div>
      </div>
    </details>
    <section class="command-output" aria-labelledby="command-title">
      <header class="extension-heading">
        <h3 id="command-title">运行命令</h3>
        <button class="button primary compact" :disabled="!fullCommand" @click="copyCommand">复制命令</button>
      </header>
      <p v-if="validation" class="inline-warning" role="alert">{{ validation }}</p>
      <template v-if="preview">
        <pre tabindex="0" aria-label="运行命令预览">{{ preview }}</pre>
        <p class="muted">在终端粘贴并执行。{{ apiKey && !reveal ? "复制的命令包含完整密钥。" : "" }}</p>
      </template>
      <p v-else-if="!version || !config.model" class="muted">
        {{ !version ? "请选择一个已安装版本。" : "请选择模型文件以生成命令。" }}
      </p>
      <p v-if="copyStatus" role="status">{{ copyStatus }}</p>
    </section>
  </section>
</template>

<style scoped>
.command-panel { min-width: 0; gap: 16px; }
.selected-version { color: var(--accent); }
.model-field { display: grid; gap: 7px; color: var(--muted); font-size: 13px; }
.model-picker, .key-field { display: flex; align-items: center; gap: 12px; }
.model-picker { padding: 8px 9px 8px 12px; border: 1px dashed var(--line-strong); border-radius: 8px; }
.model-path { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.model-picker button, .key-field button { flex-shrink: 0; }
.key-field { margin-top: -9px; }
.key-field input { flex: 1; }
.advanced-fields { margin-top: 10px; }
.advanced-field { display: grid; gap: 7px; }
.advanced-field > div { display: flex; justify-content: space-between; flex-wrap: wrap; gap: 4px; }
.advanced-field button { padding: 0; }
.command-output { border-top: 1px solid var(--line); padding-top: 16px; }
.command-output h3 { margin: 0; }
.command-output .muted { font-size: 13px; }
.command-output pre { user-select: text; line-height: 1.65; }
</style>
