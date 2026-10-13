import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTransportForTests } from "../api/transport";
import { failedRun, snapshotWithTools, updatePlan } from "../test/fixtures";
import { apiError, createMockTransport, ok } from "../test/mockTransport";
import { useRunsStore } from "./runs";
import type { ExecutionResultDto } from "../bindings";

describe("runs store", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("isolates output by run and ignores stale or duplicate chunks", () => {
    const store = useRunsStore();
    store.applyOutput({
      runId: "run-a",
      firstSequence: "1",
      lastSequence: "2",
      timestamp: "2026-08-27T10:00:00Z",
      chunks: [
        { sequence: "1", stream: "stdout", message: "a1" },
        { sequence: "2", stream: "stderr", message: "a2" },
      ],
    });
    store.applyOutput({
      runId: "run-b",
      firstSequence: "1",
      lastSequence: "1",
      timestamp: "2026-08-27T10:00:00Z",
      chunks: [{ sequence: "1", stream: "stdout", message: "b1" }],
    });
    store.applyOutput({
      runId: "run-a",
      firstSequence: "2",
      lastSequence: "2",
      timestamp: "2026-08-27T10:00:01Z",
      chunks: [{ sequence: "2", stream: "stdout", message: "duplicate" }],
    });

    expect(store.liveRuns.get("run-a")?.lines.map((line) => line.message)).toEqual(["a1", "a2"]);
    expect(store.liveRuns.get("run-b")?.lines.map((line) => line.message)).toEqual(["b1"]);
  });

  it("retains every high-frequency log line and applies final state immediately", async () => {
    vi.useFakeTimers();
    const store = useRunsStore();
    store.applyOutput({
      runId: "large-run",
      firstSequence: "1",
      lastSequence: "1000",
      timestamp: "2026-08-27T10:00:00Z",
      chunks: Array.from({ length: 1000 }, (_, index) => ({
        sequence: String(index + 1),
        stream: "stdout" as const,
        message: `line-${index}`,
      })),
    });
    store.applyState({
      runId: "large-run",
      sequence: "1001",
      timestamp: "2026-08-27T10:00:02Z",
      state: { kind: "state_changed", status: "failed" },
    });

    expect(store.liveRuns.get("large-run")?.lines).toHaveLength(1000);
    expect(store.liveRuns.get("large-run")?.status).toBe("failed");
    await vi.runAllTimersAsync();
    expect(store.liveRuns.get("large-run")?.status).toBe("failed");
  });

  it("keeps output delivered after a newer state event and never regresses the state", () => {
    const store = useRunsStore();
    store.applyState({ runId: "run", sequence: "5", timestamp: "2026-09-07T10:00:02Z", state: { kind: "state_changed", status: "succeeded" } });
    store.applyOutput({ runId: "run", firstSequence: "2", lastSequence: "3", timestamp: "2026-09-07T10:00:01Z", chunks: [
      { sequence: "2", stream: "stdout", message: "first output" },
      { sequence: "3", stream: "stderr", message: "second output" },
    ] });
    store.applyState({ runId: "run", sequence: "1", timestamp: "2026-09-07T10:00:00Z", state: { kind: "state_changed", status: "running" } });
    expect(store.liveRuns.get("run")?.lines.map((line) => line.message)).toEqual(["first output", "second output"]);
    expect(store.liveRuns.get("run")?.status).toBe("succeeded");
  });

  it("sends only plan id/hash and forces a new preview after invalid_plan", async () => {
    const plan = updatePlan();
    const confirmBatch = vi.fn(() => apiError("invalid_plan", "expired"));
    const mock = createMockTransport({
      preview: vi.fn(() => ok([plan])),
      confirmBatch,
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();
    await store.preview([
      { toolId: plan.toolId, componentId: plan.componentId, strategyId: null, targetVersion: null },
    ]);

    const result = await store.confirmPlans();

    expect(result).toEqual([]);
    expect(confirmBatch).toHaveBeenCalledWith({ plans: [{ planId: "plan-0", planHash: "hash-0" }] });
    expect(store.needsRepreview).toBe(true);
    expect(store.pendingPlans).toEqual([]);
    expect(store.pendingSelections).toHaveLength(1);
  });

  it("supports partial failure history, cancellation and retry through backend APIs", async () => {
    const run = failedRun();
    const plan = updatePlan();
    const mock = createMockTransport({
      runHistory: vi.fn(() => ok({ runs: [run] })),
      preview: vi.fn(() => ok([plan])),
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();

    await store.loadHistory();
    await store.cancel(run.id);
    await store.retry(run);

    expect(store.history[0]?.status).toBe("failed");
    expect(mock.transport.commands.cancel).toHaveBeenCalledWith({ runId: run.id });
    expect(mock.transport.commands.preview).toHaveBeenCalledWith({
      selections: [{ toolId: run.toolId, componentId: "core", strategyId: null, targetVersion: null }],
    });
  });

  it("completes a typed fake refresh-to-update flow", async () => {
    const plan = updatePlan();
    const snapshot = snapshotWithTools();
    const result = {
      runId: "run-success",
      status: "succeeded",
      exitCode: 0,
      startedAt: "2026-08-27T10:00:00Z",
      finishedAt: "2026-08-27T10:00:02Z",
      outputTail: "updated",
      verification: { status: "verification_unknown", reason: null },
    };
    const mock = createMockTransport({
      preview: vi.fn(() => ok([plan])),
      confirmBatch: vi.fn(() => ok({ outcomes: [{ status: "ok" as const, planId: plan.planId, execution: result }], snapshot })),
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();

    await store.preview([{ toolId: plan.toolId, componentId: "core", strategyId: null, targetVersion: null }]);
    const executions = await store.confirmPlans();

    expect(executions[0]).toEqual(result);
    expect(store.liveRuns.get("run-success")?.status).toBe("succeeded");
    expect(store.liveRuns.get("run-success")?.lines[0]?.message).toBe("updated");
  });

  it("submits every approved plan together and only repreviews entries that never started", async () => {
    const plans = [updatePlan(0), { ...updatePlan(1), toolId: "npm:tool-1" }, { ...updatePlan(2), toolId: "npm:tool-2" }];
    const execution: ExecutionResultDto = {
      runId: "completed-run", status: "succeeded", exitCode: 0,
      startedAt: "2026-09-11T05:25:26Z", finishedAt: "2026-09-11T05:26:37Z",
      outputTail: "updated", verification: null,
    };
    const confirmBatch = vi.fn(() => ok({
      outcomes: [
        { status: "ok" as const, planId: plans[0]!.planId, execution },
        { status: "error" as const, planId: plans[1]!.planId, error: { code: "invalid_plan" as const, message: "command unavailable", retryable: true, details: {} } },
        { status: "ok" as const, planId: plans[2]!.planId, execution: { ...execution, runId: "last-run" } },
      ],
      snapshot: snapshotWithTools(),
    }));
    const mock = createMockTransport({ preview: vi.fn(() => ok(plans)), confirmBatch });
    setTransportForTests(mock.transport);
    const store = useRunsStore();
    const selections = plans.map((plan) => ({ toolId: plan.toolId, componentId: plan.componentId, strategyId: null, targetVersion: null }));
    await store.preview(selections);
    const results = await store.confirmPlans();
    expect(confirmBatch).toHaveBeenCalledExactlyOnceWith({ plans: plans.map((plan) => ({ planId: plan.planId, planHash: plan.planHash })) });
    expect(mock.transport.commands.confirm).not.toHaveBeenCalled();
    expect(results.map((result) => result.runId)).toEqual(["completed-run", "last-run"]);
    expect(store.needsRepreview).toBe(true);
    expect(store.pendingSelections).toEqual([selections[1]]);
    await store.repreview();
    expect(mock.transport.commands.preview).toHaveBeenLastCalledWith({ selections: [selections[1]] });
  });

  it("previews fresh plans and confirms them in one update action", async () => {
    const plan = updatePlan(1);
    const execution: ExecutionResultDto = {
      runId: "one-click-run", status: "succeeded", exitCode: 0,
      startedAt: "2026-10-13T05:25:26Z", finishedAt: "2026-10-13T05:26:37Z",
      outputTail: "updated", verification: null,
    };
    const mock = createMockTransport({
      preview: vi.fn(() => ok([plan])),
      confirmBatch: vi.fn(() => ok({
        outcomes: [{ status: "ok" as const, planId: plan.planId, execution }],
        snapshot: snapshotWithTools(),
      })),
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();
    store.pendingPlans = [updatePlan(0)];
    const selections = [{ toolId: plan.toolId, componentId: plan.componentId, strategyId: null, targetVersion: null }];

    const update = store.updateNow(selections);
    expect(store.startingUpdate).toBe(true);
    expect(store.pendingPlans).toEqual([]);
    expect(await update).toEqual([execution]);

    expect(mock.transport.commands.preview).toHaveBeenCalledExactlyOnceWith({ selections });
    expect(mock.transport.commands.confirmBatch).toHaveBeenCalledExactlyOnceWith({
      plans: [{ planId: plan.planId, planHash: plan.planHash }],
    });
    expect(store.liveRuns.get(execution.runId)?.status).toBe("succeeded");
    expect(store.startingUpdate).toBe(false);
    expect(store.executing).toBe(false);
    expect(store.pendingPlans).toEqual([]);
  });

  it("does not execute an old plan when one-click preview fails", async () => {
    const plan = updatePlan();
    const mock = createMockTransport({ preview: vi.fn(() => apiError("invalid_plan", "preview unavailable")) });
    setTransportForTests(mock.transport);
    const store = useRunsStore();
    store.pendingPlans = [plan];

    await expect(store.updateNow([
      { toolId: plan.toolId, componentId: plan.componentId, strategyId: null, targetVersion: null },
    ])).rejects.toThrow("preview unavailable");

    expect(mock.transport.commands.confirmBatch).not.toHaveBeenCalled();
    expect(store.pendingPlans).toEqual([]);
    expect(store.error).toBe("preview unavailable");
    expect(store.startingUpdate).toBe(false);
    expect(store.executing).toBe(false);
  });

  it("blocks overlapping one-click updates and manual confirmation during preparation", async () => {
    const plan = updatePlan();
    let releasePreview = () => {};
    let releaseConfirmation = () => {};
    const previewGate = new Promise<void>((resolve) => { releasePreview = resolve; });
    const confirmationGate = new Promise<void>((resolve) => { releaseConfirmation = resolve; });
    const mock = createMockTransport({
      preview: vi.fn(async () => { await previewGate; return ok([plan]); }),
      confirmBatch: vi.fn(async () => {
        await confirmationGate;
        return ok({ outcomes: [], snapshot: snapshotWithTools() });
      }),
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();
    const selections = [{ toolId: plan.toolId, componentId: plan.componentId, strategyId: null, targetVersion: null }];

    const update = store.updateNow(selections);
    expect(await store.updateNow(selections)).toEqual([]);
    expect(await store.confirmPlans()).toEqual([]);
    await expect(store.preview(selections)).rejects.toThrow("正在准备或执行更新");
    expect(mock.transport.commands.preview).toHaveBeenCalledTimes(1);
    expect(mock.transport.commands.confirmBatch).not.toHaveBeenCalled();

    releasePreview();
    await vi.waitFor(() => expect(mock.transport.commands.confirmBatch).toHaveBeenCalledTimes(1));
    expect(store.executing).toBe(true);
    expect(await store.updateNow(selections)).toEqual([]);
    expect(await store.confirmPlans()).toEqual([]);
    releaseConfirmation();
    await update;

    expect(mock.transport.commands.preview).toHaveBeenCalledTimes(1);
    expect(mock.transport.commands.confirmBatch).toHaveBeenCalledTimes(1);
    expect(store.startingUpdate).toBe(false);
    expect(store.executing).toBe(false);
  });

  it("preserves repreview state and rejects when one-click confirmation cannot start", async () => {
    const plan = updatePlan();
    const mock = createMockTransport({
      preview: vi.fn(() => ok([plan])),
      confirmBatch: vi.fn(() => apiError("invalid_plan", "expired")),
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();
    const selections = [{ toolId: plan.toolId, componentId: plan.componentId, strategyId: null, targetVersion: null }];

    await expect(store.updateNow(selections)).rejects.toThrow("重新预览");

    expect(store.needsRepreview).toBe(true);
    expect(store.pendingPlans).toEqual([]);
    expect(store.pendingSelections).toEqual(selections);
    expect(store.error).toContain("重新预览");
    expect(store.startingUpdate).toBe(false);
    expect(store.executing).toBe(false);
  });
});
