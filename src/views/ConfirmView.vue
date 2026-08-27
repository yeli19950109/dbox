<template>
  <div class="view confirm-view">
    <header class="view-header">
      <div>
        <p class="eyebrow">Review plans</p>
        <h1>确认更新</h1>
        <p class="view-description">
          下列命令由后端 UpdatePlan 生成。确认时仅回传 plan id 与 hash。
        </p>
      </div>
      <RouterLink class="button secondary" to="/tools">返回工具</RouterLink>
    </header>

    <div v-if="runs.needsRepreview" class="inline-warning" role="alert">
      <div>
        <strong>计划已失效</strong>
        <p>{{ runs.error }}</p>
      </div>
      <button class="button primary" type="button" @click="repreview">
        重新预览
      </button>
    </div>

    <AppEmptyState
      v-if="!runs.pendingPlans.length"
      title="没有待确认的计划"
      description="请回到工具列表选择一个或多个可更新的 Component。"
      mark="✓"
    >
      <RouterLink class="button primary" to="/tools">选择工具</RouterLink>
    </AppEmptyState>

    <section v-else class="plan-stack" aria-label="待确认更新计划">
      <article v-for="(plan, index) in runs.pendingPlans" :key="plan.planId" class="plan-card">
        <header>
          <span class="plan-index">{{ String(index + 1).padStart(2, "0") }}</span>
          <div>
            <p class="eyebrow">{{ sourceLabel(plan) }}</p>
            <h2>{{ toolLabel(plan) }} · {{ componentLabel(plan) }}</h2>
          </div>
          <span v-if="plan.command.networkRequired" class="risk-pill">需要联网</span>
        </header>
        <dl class="plan-grid">
          <div>
            <dt>Program</dt>
            <dd><code>{{ plan.command.program }}</code></dd>
          </div>
          <div>
            <dt>Strategy</dt>
            <dd>{{ strategyLabel(plan) }}</dd>
          </div>
          <div>
            <dt>工作目录</dt>
            <dd><code>{{ plan.command.cwd ?? "继承应用目录" }}</code></dd>
          </div>
          <div>
            <dt>超时</dt>
            <dd>{{ plan.command.timeoutSeconds }} 秒</dd>
          </div>
        </dl>
        <div class="argv-block">
          <span>argv</span>
          <ol>
            <li v-for="(arg, argIndex) in plan.command.args" :key="`${argIndex}-${arg}`">
              <code>{{ arg }}</code>
            </li>
          </ol>
        </div>
        <footer>
          <span>计划有效至 {{ formatDate(plan.expiresAt) }}</span>
          <span v-if="plan.command.postCheck">完成后将重新检查版本</span>
        </footer>
      </article>
    </section>

    <section v-if="runs.pendingPlans.length" class="confirmation-bar">
      <div>
        <strong>{{ runs.pendingPlans.length }} 个计划将依次运行</strong>
        <p>包管理器可能修改全局文件；失败不会阻断后续计划。</p>
      </div>
      <button
        class="button danger"
        type="button"
        :disabled="runs.executing"
        @click="confirm"
      >
        {{ runs.executing ? "正在执行…" : "确认并运行" }}
      </button>
    </section>
  </div>
</template>

<script setup lang="ts">
import { useRouter } from "vue-router";
import type { UpdatePlanDto } from "../bindings";
import AppEmptyState from "../components/AppEmptyState.vue";
import { useRunsStore } from "../stores/runs";
import { useSnapshotStore } from "../stores/snapshot";
import { formatDate } from "../utils/presentation";

const runs = useRunsStore();
const snapshots = useSnapshotStore();
const router = useRouter();

function tool(plan: UpdatePlanDto) {
  return snapshots.snapshot?.tools.find((item) => item.id === plan.toolId);
}
function component(plan: UpdatePlanDto) {
  return tool(plan)?.components.find((item) => item.id === plan.componentId);
}
function sourceLabel(plan: UpdatePlanDto): string {
  const installation = snapshots.snapshot?.installations.find(
    (item) => item.id === plan.installationId,
  );
  return installation
    ? `${installation.providerId} · ${installation.packageKind}:${installation.packageName}`
    : plan.installationId;
}
function toolLabel(plan: UpdatePlanDto): string {
  return tool(plan)?.displayName ?? plan.toolId;
}
function componentLabel(plan: UpdatePlanDto): string {
  return component(plan)?.displayName ?? plan.componentId;
}
function strategyLabel(plan: UpdatePlanDto): string {
  return (
    component(plan)?.strategies.find((item) => item.id === plan.strategyId)?.displayName ??
    plan.strategyId
  );
}

async function confirm(): Promise<void> {
  await runs.confirmPlans();
  if (!runs.needsRepreview) await router.push({ name: "runs" });
}

async function repreview(): Promise<void> {
  await runs.repreview();
}
</script>
