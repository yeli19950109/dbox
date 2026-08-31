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
        :aria-label="`选择 ${record.tool.displayName} 的全部更新`"
        @change="$emit('select-updates')"
      />
    </span>
    <button
      class="tool-card__summary"
      type="button"
      :aria-label="`查看 ${record.tool.displayName} 详情`"
      @click="$emit('details', $event)"
    >
      <h2>{{ record.tool.displayName }}</h2>
      <span class="status-pill" :data-status="record.tool.status.status">
        {{ statusLabel(record.tool.status.status) }}
      </span>
      <span v-if="record.tool.hidden" class="quiet-pill">已隐藏</span>
      <span class="tool-card__disclosure" aria-hidden="true">›</span>
    </button>
  </article>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { ToolRecord } from "../utils/presentation";
import { updateableComponents } from "../utils/presentation";

const props = defineProps<{
  record: ToolRecord;
  selected: ReadonlySet<string>;
}>();

defineEmits<{
  details: [event: MouseEvent];
  "select-updates": [];
}>();

const updateable = computed(() => updateableComponents(props.record.tool));
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
