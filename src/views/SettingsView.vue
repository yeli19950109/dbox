<template>
  <div class="view settings-view">
    <AppLoading v-if="settings.loading && !draft" label="正在读取设置…" />
    <div v-else-if="loadError" class="inline-error" role="alert">
      <span>{{ loadError }}</span>
      <button class="text-button" type="button" @click="loadSettings">重试</button>
    </div>

    <form v-else-if="draft" class="settings-form" @submit.prevent="persistSettings">
      <section class="settings-card">
        <header>
          <div>
            <p class="eyebrow">Environment</p>
            <h2>命令与 PATH</h2>
          </div>
          <p>留空表示使用后端解析结果。</p>
        </header>
        <div class="field-grid">
          <label>
            <span>PATH 覆盖</span>
            <input v-model="draft.executableOverrides.PATH" type="text" placeholder="/opt/homebrew/bin:/usr/local/bin" />
          </label>
          <label>
            <span>npm 路径</span>
            <input v-model="draft.executableOverrides.npm" type="text" placeholder="/opt/homebrew/bin/npm" />
          </label>
          <label>
            <span>brew 路径</span>
            <input v-model="draft.executableOverrides.brew" type="text" placeholder="/opt/homebrew/bin/brew" />
          </label>
        </div>
      </section>

      <section class="settings-card">
        <header>
          <div>
            <p class="eyebrow">Providers</p>
            <h2>扫描来源</h2>
          </div>
          <p>停用后该 Provider 不参与刷新。</p>
        </header>
        <div class="toggle-grid">
          <label v-for="provider in providerIds" :key="provider" class="toggle-control">
            <span>
              <strong>{{ provider }}</strong>
              <small>{{ providerStatus(provider) }}</small>
            </span>
            <input v-model="draft.providerEnabled[provider]" type="checkbox" role="switch" />
          </label>
        </div>
      </section>

      <section class="settings-card">
        <header>
          <div>
            <p class="eyebrow">Execution & retention</p>
            <h2>运行限制</h2>
          </div>
        </header>
        <div class="field-grid three">
          <label>
            <span>默认超时（秒）</span>
            <input v-model="draft.defaultTimeoutSeconds" type="text" inputmode="numeric" pattern="[0-9]+" required />
          </label>
          <label>
            <span>最多日志文件</span>
            <input v-model.number="draft.logRetention.maxFiles" type="number" min="1" required />
          </label>
          <label>
            <span>日志总大小（字节）</span>
            <input v-model="draft.logRetention.maxTotalBytes" type="text" inputmode="numeric" pattern="[0-9]+" required />
          </label>
        </div>
      </section>

      <section v-if="strategyComponents.length" class="settings-card">
        <header>
          <div>
            <p class="eyebrow">Strategies</p>
            <h2>Component 默认策略</h2>
          </div>
          <p>只显示后端快照提供的可选策略。</p>
        </header>
        <div class="strategy-grid">
          <label v-for="item in strategyComponents" :key="`${item.tool.id}-${item.component.id}`">
            <span>{{ item.tool.displayName }} · {{ item.component.displayName }}</span>
            <select
              :value="componentStrategy(item.tool.id, item.component.id)"
              @change="setComponentStrategy(item.tool.id, item.component.id, ($event.target as HTMLSelectElement).value)"
            >
              <option
                v-for="strategy in item.component.strategies"
                :key="strategy.id"
                :value="strategy.id"
                :disabled="!strategy.enabled"
              >
                {{ strategy.displayName }}{{ strategy.enabled ? "" : "（不可用）" }}
              </option>
            </select>
          </label>
        </div>
      </section>

      <div v-if="settingsConflict" class="inline-warning" role="alert">
        <div>
          <strong>设置已在其他位置发生变化</strong>
          <p>已读取新的 revision，但保留了当前表单。检查后再次保存即可。</p>
        </div>
      </div>
      <div class="form-actions">
        <span v-if="settings.document" class="revision-chip">
          revision {{ shortRevision(settings.document.revision) }}
        </span>
        <button
          class="button primary"
          data-testid="settings-save"
          type="button"
          :disabled="settings.saving"
          @click="persistSettings"
        >
          {{ settings.saving ? "正在保存…" : "保存设置" }}
        </button>
      </div>
    </form>

    <section class="settings-card manifest-card">
      <header>
        <div>
          <p class="eyebrow">Catalog manifest</p>
          <h2>用户 Manifest</h2>
        </div>
        <span v-if="manifestRevision" class="revision-chip">revision {{ shortRevision(manifestRevision) }}</span>
      </header>
      <div class="manifest-controls">
        <label>
          <span>文件名</span>
          <input v-model="manifestFile" type="text" autocomplete="off" />
        </label>
        <button class="button secondary" type="button" @click="readManifest">读取</button>
      </div>
      <label>
        <span>内容（TOML）</span>
        <textarea v-model="manifestContents" rows="15" spellcheck="false" />
      </label>
      <div v-if="settings.validation" class="validation-result" role="status">
        <strong>校验通过</strong>
        <span>Manifest ID：{{ settings.validation.manifestId }}</span>
      </div>
      <div v-if="manifestError" class="inline-error" role="alert">{{ manifestError }}</div>
      <div v-if="manifestConflict" class="inline-warning" role="alert">
        <div>
          <strong>Manifest revision 冲突</strong>
          <p>远端 revision 已更新，编辑内容仍保留。请检查后再次保存。</p>
        </div>
      </div>
      <footer class="form-actions split">
        <button class="button secondary" type="button" @click="validateManifest">仅校验</button>
        <button class="button primary" type="button" @click="saveManifest">校验并保存</button>
      </footer>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, toRaw } from "vue";
import type { SettingsValueDto } from "../bindings";
import { BackendError, errorMessage } from "../api/transport";
import AppLoading from "../components/AppLoading.vue";
import { useNotificationsStore } from "../stores/notifications";
import { useSettingsStore } from "../stores/settings";
import { useSnapshotStore } from "../stores/snapshot";

const settings = useSettingsStore();
const snapshots = useSnapshotStore();
const notifications = useNotificationsStore();
const draft = ref<SettingsValueDto | null>(null);
const loadError = ref<string | null>(null);
const settingsConflict = ref(false);
const manifestFile = ref("custom.toml");
const manifestContents = ref("");
const manifestRevision = ref<string | null>(null);
const manifestError = ref<string | null>(null);
const manifestConflict = ref(false);

const providerIds = computed(() => [
  ...new Set([
    ...Object.keys(draft.value?.providerEnabled ?? {}),
    ...(snapshots.snapshot?.providers.map((provider) => provider.providerId) ?? []),
  ]),
]);

const strategyComponents = computed(() =>
  (snapshots.snapshot?.tools ?? []).flatMap((tool) =>
    tool.components
      .filter((component) => component.strategies.length)
      .map((component) => ({ tool, component })),
  ),
);

function cloneSettings(value: SettingsValueDto): SettingsValueDto {
  const copy = structuredClone(toRaw(value));
  copy.executableOverrides.PATH ??= "";
  copy.executableOverrides.npm ??= "";
  copy.executableOverrides.brew ??= "";
  return copy;
}

function settingsForSave(value: SettingsValueDto): SettingsValueDto {
  const copy = structuredClone(toRaw(value));
  copy.executableOverrides = Object.fromEntries(
    Object.entries(copy.executableOverrides)
      .map(([name, path]) => [name, path.trim()] as const)
      .filter(([, path]) => path.length > 0),
  );
  return copy;
}

async function loadSettings(preserveDraft = false): Promise<void> {
  loadError.value = null;
  try {
    const document = await settings.load();
    if (!preserveDraft) draft.value = cloneSettings(document.settings);
  } catch (reason) {
    loadError.value = errorMessage(reason, "无法连接 dbox 后端并读取设置。");
  }
}

async function persistSettings(): Promise<void> {
  if (!draft.value) throw new Error("设置草稿尚未加载");
  settingsConflict.value = false;
  try {
    const document = await settings.save(settingsForSave(draft.value));
    draft.value = cloneSettings(document.settings);
    notifications.push("success", "设置已保存");
  } catch (reason) {
    if (reason instanceof BackendError && reason.code === "conflict") {
      settingsConflict.value = true;
      await loadSettings(true);
      return;
    }
    notifications.push("error", "设置保存失败", reason instanceof Error ? reason.message : String(reason));
  }
}

function providerStatus(providerId: string): string {
  const provider = snapshots.snapshot?.providers.find((item) => item.providerId === providerId);
  if (!provider) return "等待刷新";
  if (!provider.status?.available) return provider.status?.detail ?? "不可用";
  return provider.status.version ?? "可用";
}

function componentStrategy(toolId: string, componentId: string): string {
  const component = snapshots.snapshot?.tools
    .find((tool) => tool.id === toolId)
    ?.components.find((item) => item.id === componentId);
  return (
    draft.value?.componentStrategies[toolId]?.[componentId] ??
    component?.defaultStrategy ??
    component?.strategies.find((strategy) => strategy.enabled)?.id ??
    ""
  );
}

function setComponentStrategy(toolId: string, componentId: string, strategyId: string): void {
  if (!draft.value) return;
  draft.value.componentStrategies[toolId] = {
    ...draft.value.componentStrategies[toolId],
    [componentId]: strategyId,
  };
}

async function readManifest(): Promise<void> {
  manifestError.value = null;
  manifestConflict.value = false;
  try {
    const result = await settings.readManifest(manifestFile.value);
    manifestContents.value = result.contents ?? "";
    manifestRevision.value = result.revision;
  } catch (reason) {
    manifestError.value = reason instanceof Error ? reason.message : String(reason);
  }
}

async function validateManifest(): Promise<void> {
  manifestError.value = null;
  try {
    await settings.validateManifest({
      fileName: manifestFile.value,
      contents: manifestContents.value,
    });
  } catch (reason) {
    manifestError.value = reason instanceof Error ? reason.message : String(reason);
  }
}

async function saveManifest(): Promise<void> {
  manifestError.value = null;
  manifestConflict.value = false;
  try {
    await settings.saveManifest({
      fileName: manifestFile.value,
      contents: manifestContents.value,
      expectedRevision: manifestRevision.value,
    });
    manifestRevision.value = settings.manifest?.revision ?? null;
    manifestContents.value = settings.manifest?.contents ?? manifestContents.value;
    notifications.push("success", "Manifest 已保存", "工具快照已使用新 catalog 重新加载。 ");
  } catch (reason) {
    if (reason instanceof BackendError && reason.code === "conflict") {
      const localContents = manifestContents.value;
      await readManifest();
      manifestContents.value = localContents;
      manifestConflict.value = true;
      return;
    }
    manifestError.value = reason instanceof Error ? reason.message : String(reason);
  }
}

function shortRevision(value: string): string {
  return value.length > 10 ? `${value.slice(0, 10)}…` : value;
}

onMounted(() => void loadSettings());
</script>
