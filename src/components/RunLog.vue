<template>
  <section class="log-shell" aria-label="运行日志">
    <header>
      <div>
        <p class="eyebrow">Live output</p>
        <h2>输出</h2>
      </div>
      <div class="log-header-actions">
        <button
          v-if="hasMergedProgress"
          class="text-button"
          type="button"
          :aria-expanded="showFullOutput"
          @click="showFullOutput = !showFullOutput"
        >{{ showFullOutput ? "合并进度" : "展开进度" }}</button>
        <span>{{ displayedLines.length }} 条</span>
      </div>
    </header>
    <div
      ref="scrollElement"
      class="log-viewport"
      role="log"
      aria-live="polite"
      aria-relevant="additions text"
      tabindex="0"
      data-testid="log-viewport"
    >
      <p v-if="!displayedLines.length" class="log-empty">{{ emptyMessage }}</p>
      <div v-else class="virtual-list__sizer" :style="{ height: `${virtualizer.getTotalSize()}px` }">
        <div
          v-for="virtualRow in virtualRows"
          :key="displayedLines[virtualRow.index]?.displayKey"
          :ref="measureRow"
          :data-index="virtualRow.index"
          class="log-line virtual-list__row"
          :data-stream="displayedLines[virtualRow.index]?.stream"
          :style="{ transform: `translateY(${virtualRow.start}px)` }"
        >
          <span class="log-line__sequence">{{ displayedLines[virtualRow.index]?.sequence }}</span>
          <span
            class="log-line__stream"
            :title="displayedLines[virtualRow.index]?.stream === 'stderr' ? '诊断输出通道，不代表运行失败' : undefined"
          >{{ displayedLines[virtualRow.index]?.stream }}</span>
          <pre>{{ displayedLines[virtualRow.index]?.message }}</pre>
        </div>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch, type ComponentPublicInstance } from "vue";
import { useVirtualizer } from "@tanstack/vue-virtual";
import type { DisplayLogLine } from "../stores/runs";
import { compactProgressLines } from "../utils/progressLog";

const props = defineProps<{
  lines: DisplayLogLine[];
  revision: number;
  status?: string;
}>();
const scrollElement = ref<HTMLElement | null>(null);
const following = ref(true);
const showFullOutput = ref(false);
const compactedLines = computed(() => compactProgressLines(props.lines));
const hasMergedProgress = computed(() => compactedLines.value.some((line) => (line.mergedFrames ?? 1) > 1));
const displayedLines = computed(() => showFullOutput.value
  ? props.lines.map((line) => ({ ...line, displayKey: `${line.sequence}:0` }))
  : compactedLines.value);
const emptyMessage = computed(() => {
  if (props.status === "queued") return "正在排队，命令启动后会显示执行详情。";
  if (props.status === "running") return "命令正在运行，等待输出…";
  return "该运行没有输出日志。";
});

const virtualizer = useVirtualizer(
  computed(() => ({
    count: displayedLines.value.length,
    getItemKey: (index: number) => displayedLines.value[index]!.displayKey,
    getScrollElement: () => scrollElement.value,
    estimateSize: () => 28,
    overscan: 12,
    initialRect: { width: 900, height: 520 },
  })),
);
const virtualRows = computed(() => virtualizer.value.getVirtualItems());

function measureRow(element: Element | ComponentPublicInstance | null): void {
  if (element instanceof HTMLElement) virtualizer.value.measureElement(element);
}

watch(
  [() => props.revision, showFullOutput],
  () => {
    if (!following.value || displayedLines.value.length === 0) return;
    void nextTick(() => virtualizer.value.scrollToIndex(displayedLines.value.length - 1, { align: "end" }));
  },
);

function updateFollowing(): void {
  const element = scrollElement.value;
  if (!element) return;
  following.value = element.scrollHeight - element.scrollTop - element.clientHeight < 56;
}

watch(scrollElement, (element, previous) => {
  previous?.removeEventListener("scroll", updateFollowing);
  element?.addEventListener("scroll", updateFollowing, { passive: true });
});
</script>
