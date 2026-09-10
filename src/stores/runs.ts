import { computed, ref, shallowRef } from "vue";
import { defineStore } from "pinia";
import { useThrottleFn } from "@vueuse/core";
import type {
  ExecutionResultDto,
  RunDto,
  RunOutputEventDto,
  RunStateEventDto,
  UpdatePlanDto,
  UpdateSelectionDto,
} from "../bindings";
import { BackendError, getTransport, unwrapCommand } from "../api/transport";
import { isNewerSequence } from "../utils/sequence";
import { useNotificationsStore } from "./notifications";
import { useSnapshotStore } from "./snapshot";

export type DisplayLogLine = {
  sequence: string;
  timestamp: string;
  stream: string;
  message: string;
};

export type LiveRun = {
  id: string;
  status: string;
  lastStateSequence?: string;
  exitCode: number | null;
  startedAt: string | null;
  finishedAt: string | null;
  verification: ExecutionResultDto["verification"];
  lines: DisplayLogLine[];
};

function runtimeRun(id: string, timestamp: string): LiveRun {
  return {
    id,
    status: "running",
    exitCode: null,
    startedAt: timestamp,
    finishedAt: null,
    verification: null,
    lines: [],
  };
}

export const useRunsStore = defineStore("runs", () => {
  const history = ref<RunDto[]>([]);
  const liveRuns = shallowRef(new Map<string, LiveRun>());
  const pendingPlans = ref<UpdatePlanDto[]>([]);
  const pendingSelections = ref<UpdateSelectionDto[]>([]);
  const needsRepreview = ref(false);
  const loadingHistory = ref(false);
  const executing = ref(false);
  const error = ref<string | null>(null);
  const logRevision = ref(0);
  let eventCleanup: Array<() => void> = [];
  let eventsConnected = false;
  let connectingEvents: Promise<void> | null = null;

  const orderedLiveRuns = computed(() =>
    [...liveRuns.value.values()].sort((left, right) =>
      (right.startedAt ?? "").localeCompare(left.startedAt ?? ""),
    ),
  );

  const scheduleLogRender = useThrottleFn(
    () => {
      logRevision.value += 1;
    },
    50,
    { trailing: true },
  );

  function mutateRun(id: string, timestamp: string): LiveRun {
    const copy = new Map(liveRuns.value);
    const current = copy.get(id) ?? runtimeRun(id, timestamp);
    const next = { ...current, lines: [...current.lines] };
    copy.set(id, next);
    liveRuns.value = copy;
    return next;
  }

  function applyOutput(event: RunOutputEventDto): void {
    const run = mutateRun(event.runId, event.timestamp);
    const known = new Set(run.lines.map((line) => line.sequence));
    let changed = false;
    for (const chunk of event.chunks) {
      if (known.has(chunk.sequence)) continue;
      run.lines.push({
        sequence: chunk.sequence,
        timestamp: event.timestamp,
        stream: chunk.stream,
        message: chunk.message,
      });
      known.add(chunk.sequence);
      changed = true;
    }
    if (changed) {
      run.lines.sort((left, right) => BigInt(left.sequence) < BigInt(right.sequence) ? -1 : 1);
      void scheduleLogRender();
    }
  }

  function applyState(event: RunStateEventDto): void {
    const existing = liveRuns.value.get(event.runId);
    if (existing && !isNewerSequence(event.sequence, existing.lastStateSequence)) return;
    const run = mutateRun(event.runId, event.timestamp);
    run.lastStateSequence = event.sequence;
    if (event.state.kind === "state_changed") {
      run.status = event.state.status;
      if (event.state.status !== "queued" && !run.startedAt) {
        run.startedAt = event.timestamp;
      }
      if (["succeeded", "failed", "cancelled", "timed_out", "interrupted", "partial"].includes(event.state.status)) {
        run.finishedAt = event.timestamp;
      }
    } else {
      run.exitCode = event.state.code;
    }
    // Final state changes bypass log throttling; throttling is only a paint optimization.
    logRevision.value += 1;
  }

  function connectEvents(): Promise<void> {
    if (connectingEvents) return connectingEvents;
    if (eventsConnected) return Promise.resolve();
    const transport = getTransport();
    connectingEvents = Promise.allSettled([
      transport.events.runOutput.listen(({ payload }) => applyOutput(payload)),
      transport.events.runState.listen(({ payload }) => applyState(payload)),
    ]).then((results) => {
      const cleanups = results.flatMap((result) => result.status === "fulfilled" ? [result.value] : []);
      const failure = results.find((result) => result.status === "rejected");
      if (failure) {
        cleanups.forEach((cleanup) => cleanup());
        throw failure.reason;
      }
      eventCleanup = cleanups;
      eventsConnected = true;
    }).finally(() => { connectingEvents = null; });
    return connectingEvents;
  }

  function disconnectEvents(): void {
    eventCleanup.forEach((cleanup) => cleanup());
    eventCleanup = [];
    eventsConnected = false;
  }

  async function loadHistory(): Promise<RunDto[]> {
    loadingHistory.value = true;
    try {
      const result = await unwrapCommand(getTransport().commands.runHistory());
      history.value = result.runs;
      return result.runs;
    } finally {
      loadingHistory.value = false;
    }
  }

  async function loadLog(runId: string): Promise<DisplayLogLine[]> {
    const result = await unwrapCommand(
      getTransport().commands.runLog({ runId }),
    );
    const timestamp = result.entries[0]?.timestamp ?? new Date().toISOString();
    const run = mutateRun(runId, timestamp);
    const known = new Set(run.lines.map((line) => line.sequence));
    for (const entry of result.entries) {
      if (known.has(entry.sequence)) continue;
      run.lines.push(entry);
      known.add(entry.sequence);
    }
    run.lines.sort((left, right) => {
      const a = BigInt(left.sequence);
      const b = BigInt(right.sequence);
      return a < b ? -1 : a > b ? 1 : 0;
    });
    logRevision.value += 1;
    return run.lines;
  }

  async function preview(selections: UpdateSelectionDto[]): Promise<UpdatePlanDto[]> {
    error.value = null;
    needsRepreview.value = false;
    pendingSelections.value = selections;
    try {
      const plans = await unwrapCommand(
        getTransport().commands.preview({ selections }),
      );
      pendingPlans.value = plans;
      return plans;
    } catch (reason) {
      error.value = reason instanceof Error ? reason.message : String(reason);
      throw reason;
    }
  }

  function recordExecution(execution: ExecutionResultDto): void {
    const run = mutateRun(execution.runId, execution.startedAt);
    run.status = execution.status;
    run.exitCode = execution.exitCode;
    run.startedAt = execution.startedAt;
    run.finishedAt = execution.finishedAt;
    run.verification = execution.verification;
    if (execution.outputTail && run.lines.length === 0) {
      run.lines.push({
        sequence: "0",
        timestamp: execution.finishedAt,
        stream: "system",
        message: execution.outputTail,
      });
    }
    logRevision.value += 1;
  }

  function hydrateHistoryRun(historyRun: RunDto): void {
    // A history request can finish after newer live events have already arrived.
    if (liveRuns.value.get(historyRun.id)?.lastStateSequence) return;
    const run = mutateRun(historyRun.id, historyRun.startedAt ?? historyRun.createdAt);
    run.status = historyRun.status;
    run.exitCode = historyRun.summary?.exitCode ?? null;
    run.startedAt = historyRun.startedAt;
    run.finishedAt = historyRun.finishedAt;
    run.verification = historyRun.summary?.verification ?? null;
    logRevision.value += 1;
  }

  async function confirmPlans(): Promise<ExecutionResultDto[]> {
    if (executing.value || pendingPlans.value.length === 0) return [];
    executing.value = true;
    error.value = null;
    const results: ExecutionResultDto[] = [];
    const plans = [...pendingPlans.value];
    try {
      try {
        await connectEvents();
      } catch (reason) {
        error.value = `无法连接实时日志，请重试：${reason instanceof Error ? reason.message : String(reason)}`;
        return results;
      }
      for (const plan of plans) {
        try {
          // The backend plan id/hash are the only execution inputs by design.
          const response = await unwrapCommand(
            getTransport().commands.confirm({
              planId: plan.planId,
              planHash: plan.planHash,
            }),
          );
          results.push(response.execution);
          recordExecution(response.execution);
          useSnapshotStore().acceptSnapshot(response.snapshot);
        } catch (reason) {
          if (reason instanceof BackendError && reason.code === "invalid_plan") {
            needsRepreview.value = true;
            pendingPlans.value = [];
            error.value = "计划已过期或环境已变化，请重新预览后再确认。";
            useNotificationsStore().push("warning", "需要重新预览", error.value);
            break;
          }
          error.value = reason instanceof Error ? reason.message : String(reason);
          useNotificationsStore().push("error", "更新未能启动", error.value);
        }
      }
      if (!needsRepreview.value) pendingPlans.value = [];
      await loadHistory().catch(() => undefined);
      return results;
    } finally {
      executing.value = false;
    }
  }

  async function repreview(): Promise<UpdatePlanDto[]> {
    if (pendingSelections.value.length === 0) return [];
    return preview(pendingSelections.value);
  }

  async function cancel(runId: string): Promise<void> {
    await unwrapCommand(getTransport().commands.cancel({ runId }));
    useNotificationsStore().push("info", "已请求取消", `运行 ${runId}`);
  }

  async function retry(run: RunDto): Promise<UpdatePlanDto[]> {
    if (!run.toolId) return [];
    const toolId = run.toolId;
    return preview(
      run.componentIds.map((componentId) => ({
        toolId,
        componentId,
        strategyId: null,
        targetVersion: null,
      })),
    );
  }

  async function copyLog(runId: string): Promise<void> {
    const result = await unwrapCommand(
      getTransport().commands.runLog({ runId }),
    );
    const text = result.entries
      .map((entry) => `[${entry.timestamp}] ${entry.stream}: ${entry.message}`)
      .join("\n");
    await navigator.clipboard.writeText(text);
    useNotificationsStore().push("success", "日志已复制", "复制内容来自后端脱敏日志。 ");
  }

  return {
    history,
    liveRuns,
    orderedLiveRuns,
    pendingPlans,
    pendingSelections,
    needsRepreview,
    loadingHistory,
    executing,
    error,
    logRevision,
    applyOutput,
    applyState,
    connectEvents,
    disconnectEvents,
    loadHistory,
    loadLog,
    hydrateHistoryRun,
    preview,
    confirmPlans,
    repreview,
    cancel,
    retry,
    copyLog,
  };
});
