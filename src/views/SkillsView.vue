<template>
  <div class="view">
    <AppEmptyState
      title="Skills 尚未启用"
      description="dbox 不会直接读写 Skill 目录或 lock 文件。"
      mark="✦"
    />
  </div>
</template>

<script setup lang="ts">
import AppEmptyState from "../components/AppEmptyState.vue";
</script>
