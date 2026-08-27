<template>
  <div class="state-panel" role="status" aria-live="polite">
    <span class="spinner" aria-hidden="true" />
    <p>{{ label }}</p>
  </div>
</template>

<script setup lang="ts">
withDefaults(defineProps<{ label?: string }>(), { label: "正在加载…" });
</script>
