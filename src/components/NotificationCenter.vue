<template>
  <aside class="notifications" aria-label="通知" aria-live="polite">
    <article
      v-for="item in store.items"
      :key="item.id"
      class="notification"
      :data-tone="item.tone"
    >
      <div>
        <strong>{{ item.title }}</strong>
        <p v-if="item.message">{{ item.message }}</p>
      </div>
      <button
        type="button"
        class="icon-button"
        :aria-label="`关闭通知：${item.title}`"
        @click="store.dismiss(item.id)"
      >
        ×
      </button>
    </article>
  </aside>
</template>

<script setup lang="ts">
import { useNotificationsStore } from "../stores/notifications";

const store = useNotificationsStore();
</script>
