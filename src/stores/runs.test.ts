import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTransportForTests } from "../api/transport";
import { failedRun, snapshotWithTools, updatePlan } from "../test/fixtures";
import { apiError, createMockTransport, ok } from "../test/mockTransport";
import { useRunsStore } from "./runs";

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

  it("sends only plan id/hash and forces a new preview after invalid_plan", async () => {
    const plan = updatePlan();
    const confirm = vi.fn(() => apiError("invalid_plan", "expired"));
    const mock = createMockTransport({
      preview: vi.fn(() => ok([plan])),
      confirm,
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();
    await store.preview([
      { toolId: plan.toolId, componentId: plan.componentId, strategyId: null, targetVersion: null },
    ]);

    const result = await store.confirmPlans();

    expect(result).toEqual([]);
    expect(confirm).toHaveBeenCalledWith({ planId: "plan-0", planHash: "hash-0" });
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
      confirm: vi.fn(() => ok({ execution: result, snapshot })),
    });
    setTransportForTests(mock.transport);
    const store = useRunsStore();

    await store.preview([{ toolId: plan.toolId, componentId: "core", strategyId: null, targetVersion: null }]);
    const executions = await store.confirmPlans();

    expect(executions[0]).toEqual(result);
    expect(store.liveRuns.get("run-success")?.status).toBe("succeeded");
    expect(store.liveRuns.get("run-success")?.lines[0]?.message).toBe("updated");
  });
});
