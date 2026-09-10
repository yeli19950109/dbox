<template>
  <div v-if="visible && operations.error" class="inline-error" role="alert">
    {{ operations.error }}
  </div>
  <div
    v-if="visible && operations.preparing"
    class="inline-warning"
    role="status"
  >
    正在准备变更预览…
  </div>
  <section
    v-if="visible && operations.plan"
    class="extension-panel operation-preview"
    aria-label="确认文件变更"
  >
    <header class="extension-heading">
      <div>
        <p class="eyebrow">确认操作 · {{ operations.plan.resource }}</p>
        <h2>
          {{ operations.plan.operation }} ·
          {{ operations.plan.names.join("、") }}
        </h2>
      </div>
      <button class="text-button" @click="operations.plan = null">
        关闭预览
      </button>
    </header>
    <p>
      确认前不会修改应用文件。计划到期时间：{{
        new Date(operations.plan.expiresAt).toLocaleTimeString()
      }}
    </p>
    <p
      v-for="warning in operations.plan.warnings"
      :key="warning"
      class="inline-warning"
    >
      {{ warning }}
    </p>
    <article
      v-for="(step, index) in operations.plan.steps"
      :key="index"
      class="operation-step"
    >
      <strong>{{ step.targetId }} · {{ step.action }}</strong
      ><code>{{ step.path }}</code>
      <p v-if="step.conflict" class="inline-error">
        冲突（此项会跳过）：{{ step.conflict }}
      </p>
      <div class="diff-columns">
        <div>
          <small>变更前</small>
          <pre>{{ step.before }}</pre>
        </div>
        <div>
          <small>变更后</small>
          <pre>{{ step.after }}</pre>
        </div>
      </div>
    </article>
    <div class="extension-actions">
      <button class="button secondary" @click="operations.plan = null">
        取消</button
      ><button
        class="button primary"
        :disabled="operations.pending"
        @click="operations.confirm"
      >
        确认执行{{
          operations.plan.steps.some((s) => s.conflict) ? "无冲突项" : ""
        }}
      </button>
    </div>
  </section>
  <section v-if="operations.pending" class="extension-panel" role="status">
    <span class="spinner" /> 正在执行，可在运行记录查看进度。<button
      class="button secondary"
      @click="operations.cancel"
    >
      取消运行</button
    ><RouterLink class="button secondary" to="/runs">运行记录</RouterLink>
  </section>
  <section
    v-if="visible && operations.result && !operations.pending"
    class="extension-panel"
    aria-label="操作结果"
  >
    <header class="extension-heading">
      <h2>{{ statusLabel(operations.result.status) }}</h2>
      <RouterLink class="text-button" to="/runs">查看运行记录</RouterLink>
    </header>
    <p
      v-for="(target, index) in operations.result.targets"
      :key="index"
      :class="
        ['applied', 'unchanged'].includes(target.status) ? '' : 'inline-warning'
      "
    >
      {{ target.targetId }} · {{ target.status }}：{{ target.message }}
    </p>
    <p v-if="operations.result.status !== 'succeeded'">
      已重新读取实际状态；可在资源列表重试未完成目标，或从备份恢复。
    </p>
  </section>
</template>
<script setup lang="ts">
import { computed } from "vue";
import type { ResourceKind } from "../../bindings";
import { useExtensionOperations } from "../../stores/extensions";
const props = defineProps<{ resource?: ResourceKind }>();
const operations = useExtensionOperations();
const visible = computed(
  () => !props.resource || operations.resource === props.resource,
);
function statusLabel(status: string) {
  return (
    (
      {
        succeeded: "操作完成",
        partial: "部分完成",
        failed: "操作失败",
        cancelled: "已取消",
        interrupted: "操作被中断",
      } as Record<string, string>
    )[status] ?? status
  );
}
</script>
