<template>
  <div class="tool-filters">
    <fieldset>
      <legend>状态</legend>
      <label v-for="option in statuses" :key="option.value" class="filter-option">
        <input v-model="status" type="radio" name="tool-status" :value="option.value" />
        <span>{{ option.label }}</span>
      </label>
    </fieldset>
    <fieldset>
      <legend>安装来源</legend>
      <label class="filter-option">
        <input v-model="provider" type="radio" name="tool-provider" value="all" />
        <span>全部来源</span>
      </label>
      <label v-for="id in providers" :key="id" class="filter-option">
        <input v-model="provider" type="radio" name="tool-provider" :value="id" />
        <span>{{ providerLabel(id) }}</span>
        <span class="filter-option__count">{{ providerCounts[id] ?? 0 }}</span>
      </label>
    </fieldset>
    <label class="filter-category">
      <span>类别</span>
      <select v-model="category">
        <option value="all">全部类别</option>
        <option v-for="item in categories" :key="item" :value="item">{{ item }}</option>
      </select>
    </label>
    <label class="check-control filter-hidden">
      <input v-model="includeHidden" type="checkbox" />
      显示已隐藏
    </label>
  </div>
</template>

<script setup lang="ts">
import { providerLabel } from "../utils/presentation";

defineProps<{
  providers: string[];
  providerCounts: Record<string, number>;
  categories: string[];
}>();

const status = defineModel<string>("status", { required: true });
const provider = defineModel<string>("provider", { required: true });
const category = defineModel<string>("category", { required: true });
const includeHidden = defineModel<boolean>("includeHidden", { required: true });
const statuses = [
  { value: "all", label: "全部状态" },
  { value: "updates", label: "有更新" },
  { value: "unknown", label: "未知" },
  { value: "failed", label: "失败" },
];
</script>
