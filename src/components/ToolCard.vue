<template>
  <article
    class="tool-card"
    :data-status="record.tool.status.status"
    :data-selected="allUpdatesSelected || undefined"
  >
    <span class="tool-card__selection">
      <input
        v-if="updateable.length"
        type="checkbox"
        :checked="allUpdatesSelected"
        :aria-label="`选择 ${summaryName} 的全部更新`"
        @change="$emit('select-updates')"
      />
    </span>
    <button
      class="tool-card__summary"
      type="button"
      :aria-label="`查看 ${summaryName} 详情`"
      @click="$emit('details', $event)"
    >
      <div class="tool-card__name">
        <h2>{{ record.tool.displayName }}</h2>
        <span v-if="piExtensionVersion" class="tool-card__version" :title="piExtensionVersion">
          {{ piExtensionVersion }}
        </span>
        <span
          v-for="installation in miseInstallations"
          :key="installation.id"
          class="tool-card__version"
          :title="miseInstallationLabel(installation)"
        >
          {{ miseInstallationLabel(installation) }}
        </span>
      </div>
      <span class="tool-card__sources" aria-label="安装来源">
        <span v-for="provider in record.providerIds" :key="provider" class="source-badge" :data-provider="providerLabel(provider)">
          {{ providerLabel(provider) }}
        </span>
        <span v-if="!record.providerIds.length" class="source-badge">来源待刷新</span>
      </span>
      <span v-if="showStatus || record.tool.hidden" class="tool-card__statuses">
        <span v-if="showStatus" class="status-pill" :data-status="record.tool.status.status">
          {{ statusLabel(record.tool.status.status) }}
        </span>
        <span v-if="record.tool.hidden" class="quiet-pill">已隐藏</span>
      </span>
      <span class="tool-card__disclosure" aria-hidden="true">›</span>
    </button>
    <button
      v-if="updateable.length"
      class="button tool-card__update"
      type="button"
      :disabled="updateDisabled"
      :aria-label="`更新 ${summaryName}`"
      @click="$emit('update')"
    >
      更新
    </button>
  </article>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { ToolRecord } from "../utils/presentation";
import { miseInstallationLabel, providerLabel, updateableComponents } from "../utils/presentation";

const props = defineProps<{
  record: ToolRecord;
  selected: ReadonlySet<string>;
  updateDisabled?: boolean;
}>();

defineEmits<{
  details: [event: MouseEvent];
  "select-updates": [];
  update: [];
}>();

const updateable = computed(() => updateableComponents(props.record.tool));
const showStatus = computed(
  () => props.record.tool.status.status !== "update_available" || !updateable.value.length,
);
const miseInstallations = computed(() =>
  props.record.installations.filter((installation) => installation.providerId === "mise"),
);
const piExtensionVersion = computed(() => {
  if (!props.record.providerIds.includes("pi-extensions")) return null;
  const component = props.record.tool.components.find((item) => item.id === "core");
  const installed = component?.installedVersion ?? "版本未知";
  return component?.status.status === "update_available" && component.latestVersion
    ? `${installed} → ${component.latestVersion}` : installed;
});
const summaryName = computed(() =>
  [props.record.tool.displayName, ...miseInstallations.value.map(miseInstallationLabel)].join(" "),
);
const allUpdatesSelected = computed(
  () =>
    updateable.value.length > 0 &&
    updateable.value.every((component) =>
      props.selected.has(`${props.record.tool.id}\u0000${component.id}`),
    ),
);

function statusLabel(status: string): string {
  const labels: Record<string, string> = {
    up_to_date: "已是最新",
    update_available: "有更新",
    unknown: "未知",
    unsupported: "不支持检查",
    checking: "检查中",
    updating: "更新中",
    check_failed: "检查失败",
    update_succeeded: "更新成功",
    update_failed: "更新失败",
    cancelled: "已取消",
    partial: "部分未知",
    mixed: "部分状态",
  };
  return labels[status] ?? status;
}
</script>

<style scoped>
.tool-card {
  position: relative;
  padding-right: 33px;
}

.tool-card__summary {
  grid-template-columns: minmax(0, 1fr) auto;
}

.tool-card__summary:has(.tool-card__statuses) {
  grid-template-columns: minmax(0, 1fr) auto auto;
}

.tool-card__disclosure {
  position: absolute;
  inset: 0 7px 0 auto;
  width: 22px;
  display: grid;
  place-items: center;
}

.tool-card__sources:has([data-provider="Pi Extensions"]) {
  width: auto;
}

.tool-card:has(.tool-card__update) {
  grid-template-columns: 36px minmax(0, 1fr) auto;
}

.tool-card__update {
  min-height: 30px;
  padding: 4px 10px;
  margin-left: 4px;
  color: var(--accent-ink);
  background: var(--accent);
  font-size: 12px;
  white-space: nowrap;
}
</style>
