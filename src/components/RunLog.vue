<template>
  <section class="log-shell" aria-label="运行日志">
    <header>
      <div>
        <p class="eyebrow">Live output</p>
        <h2>输出</h2>
      </div>
      <span>{{ lines.length }} 行</span>
    </header>
    <div
      ref="scrollElement"
      class="log-viewport"
      role="log"
      aria-live="polite"
      aria-relevant="additions"
      tabindex="0"
      data-testid="log-viewport"
    >
      <div class="virtual-list__sizer" :style="{ height: `${virtualizer.getTotalSize()}px` }">
        <div
          v-for="virtualRow in virtualRows"
          :key="lines[virtualRow.index]?.sequence"
          class="log-line virtual-list__row"
          :data-stream="lines[virtualRow.index]?.stream"
          :style="{ transform: `translateY(${virtualRow.start}px)` }"
        >
          <span class="log-line__sequence">{{ lines[virtualRow.index]?.sequence }}</span>
          <span class="log-line__stream">{{ lines[virtualRow.index]?.stream }}</span>
          <pre>{{ lines[virtualRow.index]?.message }}</pre>
        </div>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { useVirtualizer } from "@tanstack/vue-virtual";
import type { DisplayLogLine } from "../stores/runs";

const props = defineProps<{
  lines: DisplayLogLine[];
  revision: number;
}>();
const scrollElement = ref<HTMLElement | null>(null);
const following = ref(true);

const virtualizer = useVirtualizer(
  computed(() => ({
    count: props.lines.length,
    getScrollElement: () => scrollElement.value,
    estimateSize: () => 28,
    overscan: 12,
    initialRect: { width: 900, height: 520 },
  })),
);
const virtualRows = computed(() => virtualizer.value.getVirtualItems());

watch(
  () => props.revision,
  () => {
    if (!following.value || props.lines.length === 0) return;
    void nextTick(() => virtualizer.value.scrollToIndex(props.lines.length - 1, { align: "end" }));
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
