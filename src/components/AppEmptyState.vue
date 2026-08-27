<template>
  <section class="empty-state">
    <span class="empty-state__mark" aria-hidden="true">{{ mark }}</span>
    <h2>{{ title }}</h2>
    <p>{{ description }}</p>
    <slot />
  </section>
</template>

<script setup lang="ts">
withDefaults(
  defineProps<{ title: string; description: string; mark?: string }>(),
  { mark: "◇" },
);
</script>
