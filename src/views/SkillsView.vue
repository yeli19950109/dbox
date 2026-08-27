<template>
  <div class="view">
    <header class="view-header">
      <div>
        <p class="eyebrow">Future capability</p>
        <h1>Skills</h1>
        <p class="view-description">导航位置已经保留；Skill 管理将在后端 `npx skills` 适配完成后启用。</p>
      </div>
    </header>
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
