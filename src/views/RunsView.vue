<template>
  <div class="view runs-view">
    <div v-if="runs.needsRepreview" class="inline-warning" role="alert">
      <span>{{ runs.error }}</span>
      <button class="button primary" type="button" @click="repreview">重新预览</button>
    </div>
    <div v-else-if="runs.error" class="inline-error" role="alert">
      <span>{{ runs.error }}</span>
      <RouterLink v-if="runs.pendingPlans.length && !runs.executing" class="button secondary" to="/runs/confirm">返回确认</RouterLink>
    </div>
    <div v-if="runs.executing" class="execution-progress" role="status" aria-live="polite">
      <span class="spinner" aria-hidden="true" />
      <span>{{ selectedRun ? "正在执行更新，输出会实时显示在下方。" : "正在启动更新，等待执行记录…" }}</span>
    </div>
    <section v-if="runs.orderedLiveRuns.length" class="live-strip" aria-label="实时运行队列">
      <button
        v-for="run in runs.orderedLiveRuns"
        :key="run.id"
        type="button"
        class="live-run"
        :class="{ active: selectedRunId === run.id }"
        @click="selectLive(run.id)"
      >
        <span class="status-dot" :data-status="run.status" aria-hidden="true" />
        <span>
          <strong>{{ shortId(run.id) }}</strong>
          <small>{{ statusLabel(run.status) }} · {{ formatDuration(run.startedAt, run.finishedAt, now.getTime()) }}</small>
        </span>
      </button>
    </section>

    <div class="runs-layout">
      <section class="history-panel" aria-label="历史运行">
        <header>
          <h2>历史</h2>
          <span>{{ runs.history.length }} 条</span>
          <button class="text-button" type="button" @click="reloadHistory">
            刷新历史
          </button>
        </header>
        <AppLoading v-if="runs.loadingHistory" label="正在读取运行历史…" />
        <AppEmptyState
          v-else-if="!runs.history.length"
          title="还没有运行记录"
          description="从工具页选择可更新的 Component，预览并确认后会出现在这里。"
          mark="↗"
        />
        <div v-else class="history-list">
          <article
            v-for="run in runs.history"
            :key="run.id"
            class="history-item"
            :class="{ active: selectedRunId === run.id }"
          >
            <button type="button" class="history-item__main" @click="selectHistory(run)">
              <span class="status-dot" :data-status="run.status" aria-hidden="true" />
              <span>
                <strong>{{ toolName(run.toolId) }}</strong>
                <small>{{ run.componentIds.join(" · ") }}</small>
              </span>
              <span class="history-item__meta">
                <strong>{{ statusLabel(run.status) }}</strong>
                <small>{{ formatDate(run.finishedAt ?? run.createdAt) }}</small>
              </span>
            </button>
            <div class="history-item__actions">
              <button
                v-if="run.hasLog"
                type="button"
                class="text-button"
                :aria-label="`复制 ${toolName(run.toolId)} 的脱敏日志`"
                @click="copy(run.id)"
              >
                复制日志
              </button>
              <button
                v-if="canRetry(run.status)"
                type="button"
                class="text-button"
                :aria-label="`重试 ${toolName(run.toolId)}`"
                @click="retry(run)"
              >
                重试
              </button>
            </div>
          </article>
        </div>
      </section>

      <section class="run-detail" aria-label="所选运行详情">
        <template v-if="selectedRun">
          <header class="run-detail__header">
            <div>
              <p class="eyebrow">Run {{ shortId(selectedRun.id) }}</p>
              <h2>{{ statusLabel(selectedRun.status) }}</h2>
            </div>
            <div class="run-actions">
              <span>{{ formatDuration(selectedRun.startedAt, selectedRun.finishedAt, now.getTime()) }}</span>
              <button
                v-if="[&quot;queued&quot;, &quot;running&quot;].includes(selectedRun.status)"
                class="button danger compact"
                type="button"
                :aria-label="`取消运行 ${selectedRun.id}`"
                @click="cancel(selectedRun.id)"
              >
                取消运行
              </button>
            </div>
          </header>
          <div
            v-if="selectedRun.verification"
            class="verification-banner"
            :data-status="selectedRun.verification.status"
            role="status"
          >
            <strong>{{ verificationLabel(selectedRun.verification.status) }}</strong>
            <span v-if="selectedRun.verification.reason">
              {{ selectedRun.verification.reason.summary }}
            </span>
          </div>
          <RunLog :lines="selectedRun.lines" :revision="runs.logRevision" />
        </template>
        <AppLoading v-else-if="runs.executing" label="等待命令启动与实时输出…" />
        <AppEmptyState
          v-else
          title="选择一条运行记录"
          description="实时输出和后端脱敏日志会显示在这里。"
          mark="⌘"
        />
      </section>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useNow } from "@vueuse/core";
import { useRouter } from "vue-router";
import type { RunDto } from "../bindings";
import AppEmptyState from "../components/AppEmptyState.vue";
import AppLoading from "../components/AppLoading.vue";
import RunLog from "../components/RunLog.vue";
import { useRunsStore } from "../stores/runs";
import { useSnapshotStore } from "../stores/snapshot";
import { formatDate, formatDuration } from "../utils/presentation";

const runs = useRunsStore();
const snapshots = useSnapshotStore();
const router = useRouter();
const now = useNow({ interval: 1000 });
const selectedRunId = ref<string | null>(null);
const followNewRuns = ref(true);

watch(() => runs.orderedLiveRuns.map((run) => run.id), (ids, previous = []) => {
  if (!followNewRuns.value) return;
  const next = ids.find((id) => !previous.includes(id));
  if (next) selectedRunId.value = next;
}, { immediate: true });

const selectedRun = computed(() =>
  selectedRunId.value ? runs.liveRuns.get(selectedRunId.value) ?? null : null,
);

function shortId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 8)}…` : id;
}
function toolName(toolId: string): string {
  return snapshots.snapshot?.tools.find((tool) => tool.id === toolId)?.displayName ?? toolId;
}
function statusLabel(status: string): string {
  const labels: Record<string, string> = {
    queued: "排队中",
    running: "运行中",
    succeeded: "成功",
    failed: "失败",
    cancelled: "已取消",
    timed_out: "已超时",
    interrupted: "被中断",
    partial: "部分失败",
  };
  return labels[status] ?? status;
}
function canRetry(status: string): boolean {
  return ["failed", "cancelled", "timed_out", "interrupted", "partial"].includes(status);
}
function verificationLabel(status: string): string {
  const labels: Record<string, string> = {
    verified: "更新后检查通过",
    verification_unknown: "更新完成，但版本检查结果未知",
    verification_failed: "更新后检查失败",
  };
  return labels[status] ?? status;
}

function selectLive(id: string): void {
  followNewRuns.value = false;
  selectedRunId.value = id;
}
async function selectHistory(run: RunDto): Promise<void> {
  followNewRuns.value = false;
  selectedRunId.value = run.id;
  await runs.loadLog(run.id);
  runs.hydrateHistoryRun(run);
}
async function retry(run: RunDto): Promise<void> {
  await runs.retry(run);
  await router.push({ name: "confirm" });
}
async function repreview(): Promise<void> {
  await runs.repreview();
  await router.push({ name: "confirm" });
}
function cancel(runId: string): void {
  void runs.cancel(runId).catch(() => undefined);
}
function copy(runId: string): void {
  void runs.copyLog(runId).catch(() => undefined);
}
function reloadHistory(): void {
  void runs.loadHistory().catch(() => undefined);
}

onMounted(async () => {
  await runs.connectEvents().catch(() => undefined);
  await runs.loadHistory().catch(() => undefined);
  if (selectedRunId.value || runs.executing) return;
  const active = runs.orderedLiveRuns[0];
  const historical = runs.history[0];
  if (active) selectedRunId.value = active.id;
  else if (historical) {
    selectedRunId.value = historical.id;
    runs.hydrateHistoryRun(historical);
    await runs.loadLog(historical.id).catch(() => undefined);
  }
});
</script>
