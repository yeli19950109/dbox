<template>
  <section v-if="captured" class="error-boundary" role="alert">
    <p class="eyebrow">界面错误</p>
    <h2>这个区域暂时无法显示</h2>
    <p>{{ captured.message }}</p>
    <button class="button secondary" type="button" @click="captured = null">
      重试显示
    </button>
  </section>
  <slot v-else />
</template>

<script setup lang="ts">
import { onErrorCaptured, ref } from "vue";

const captured = ref<Error | null>(null);
onErrorCaptured((reason) => {
  captured.value = reason instanceof Error ? reason : new Error(String(reason));
  return false;
});
</script>
