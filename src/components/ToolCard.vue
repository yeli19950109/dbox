<template>
  <article class="tool-card" :data-status="record.tool.status.status">
    <header class="tool-card__header">
      <div>
        <div class="tool-card__title-row">
          <h2>{{ record.tool.displayName }}</h2>
          <span class="status-pill" :data-status="record.tool.status.status">
            {{ statusLabel(record.tool.status.status) }}
          </span>
          <span v-if="record.tool.hidden" class="quiet-pill">已隐藏</span>
        </div>
        <p class="tool-card__coordinate">
          {{ coordinates }}
        </p>
      </div>
      <button
        class="button secondary compact"
        type="button"
        :aria-label="`查看 ${record.tool.displayName} 详情`"
        @click="$emit('details', $event)"
      >
        详情
      </button>
    </header>

    <div class="component-list">
      <label
        v-for="component in visibleComponents"
        :key="component.id"
        class="component-row"
      >
        <input
          type="checkbox"
          :checked="selected.has(componentKey(component.id))"
          :disabled="!canUpdate(component)"
          :aria-label="`选择 ${record.tool.displayName} 的 ${component.displayName}`"
          @change="$emit('toggle', component.id)"
        />
        <span class="component-row__name">{{ component.displayName }}</span>
        <span class="version-pair">
          <span>{{ component.installedVersion ?? "版本未知" }}</span>
          <span aria-hidden="true">→</span>
          <strong :class="{ muted: !component.latestVersion }">
            {{ latestLabel(component) }}
          </strong>
        </span>
        <span class="component-state">
          {{ componentStatusLabel(component.status.status) }}
        </span>
      </label>
      <p v-if="record.tool.components.length > visibleComponents.length" class="component-overflow">
        另有 {{ record.tool.components.length - visibleComponents.length }} 个 Component，请在详情中查看
      </p>
    </div>

    <footer class="tool-card__footer">
      <span>{{ record.tool.components.length }} 个 Component</span>
      <span>{{ executableCount }} 个 Executable</span>
      <button
        v-if="record.tool.status.hasUpdates"
        type="button"
        class="text-button"
        @click="$emit('select-updates')"
      >
        选择全部更新
      </button>
      <button
        type="button"
        class="icon-button refresh-button"
        :aria-label="`刷新 ${record.tool.displayName}`"
        @click="$emit('refresh')"
      >
        ↻
      </button>
    </footer>
  </article>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { ComponentDto } from "../bindings";
import type { ToolRecord } from "../utils/presentation";

const props = defineProps<{
  record: ToolRecord;
  selected: ReadonlySet<string>;
}>();

defineEmits<{
  details: [event: MouseEvent];
  toggle: [componentId: string];
  "select-updates": [];
  refresh: [];
}>();

const coordinates = computed(() =>
  props.record.installations.length
    ? props.record.installations
        .map(
          (item) =>
            `${item.providerId} · ${item.packageKind}:${item.packageName}`,
        )
        .join("  /  ")
    : "通用工具 · 未关联安装记录",
);

const executableCount = computed(() =>
  props.record.installations.reduce(
    (total, installation) => total + installation.executables.length,
    0,
  ),
);
const visibleComponents = computed(() => props.record.tool.components.slice(0, 2));

function componentKey(componentId: string): string {
  return `${props.record.tool.id}\u0000${componentId}`;
}

function canUpdate(component: ComponentDto): boolean {
  return (
    component.status.status === "update_available" &&
    component.strategies.some((strategy) => strategy.enabled)
  );
}

function latestLabel(component: ComponentDto): string {
  if (component.latestVersion) return component.latestVersion;
  if (component.status.status === "unsupported") return "不支持比较";
  return "尚未确认";
}

function statusLabel(status: string): string {
  const labels: Record<string, string> = {
    up_to_date: "已是最新",
    update_available: "有更新",
    unknown: "未知",
    check_failed: "检查失败",
    update_succeeded: "更新成功",
    update_failed: "更新失败",
    cancelled: "已取消",
    mixed: "部分状态",
  };
  return labels[status] ?? status;
}

function componentStatusLabel(status: string): string {
  return statusLabel(status);
}
</script>
