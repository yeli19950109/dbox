import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SnapshotDto } from "../bindings";
import { setTransportForTests } from "../api/transport";
import { createMockTransport, ok } from "../test/mockTransport";
import { snapshotWithTools } from "../test/fixtures";
import { useSnapshotStore } from "./snapshot";

describe("snapshot store", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("initializes once from the typed snapshot command", async () => {
    const mock = createMockTransport();
    setTransportForTests(mock.transport);
    const store = useSnapshotStore();

    const [first, second] = await Promise.all([store.initialize(), store.initialize()]);

    expect(first).toBe(second);
    expect(mock.transport.commands.snapshot).toHaveBeenCalledTimes(1);
    expect(store.toolRecords).toHaveLength(3);
  });

  it("deduplicates refresh by scope while preserving different scopes", async () => {
    let resolveRefresh!: (value: ReturnType<typeof ok<SnapshotDto>>) => void;
    const pending = new Promise<ReturnType<typeof ok<SnapshotDto>>>((resolve) => {
      resolveRefresh = resolve;
    }).then((value) => value);
    const refresh = vi.fn(() => pending);
    const mock = createMockTransport({ refresh });
    setTransportForTests(mock.transport);
    const store = useSnapshotStore();

    const first = store.refresh({ scope: "all" });
    const duplicate = store.refresh({ scope: "all" });
    const provider = store.refresh({ scope: "provider", providerId: "npm" });
    resolveRefresh(ok(snapshotWithTools()));
    await Promise.all([first, duplicate, provider]);

    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("drops stale refresh progress events independently per request", () => {
    const store = useSnapshotStore();
    store.applyRefreshProgress({ requestId: "a", sequence: "2", phase: "completed", message: "done", providerId: null, completed: 2, total: 2 });
    store.applyRefreshProgress({ requestId: "a", sequence: "1", phase: "started", message: "stale", providerId: null, completed: null, total: null });
    store.applyRefreshProgress({ requestId: "b", sequence: "1", phase: "failed", message: "partial", providerId: null, completed: 0, total: 1 });

    expect(store.refreshProgress).toHaveLength(2);
    expect(store.refreshProgress.find((event) => event.requestId === "a")?.message).toBe("done");
  });

  it("tracks the current Provider progress event", () => {
    const store = useSnapshotStore();
    store.applyRefreshProgress({
      requestId: "refresh-1",
      sequence: "3",
      phase: "progress",
      message: "checking homebrew",
      providerId: "homebrew",
      completed: 1,
      total: 2,
    });

    expect(store.currentRefreshProgress?.providerId).toBe("homebrew");
    expect(store.currentRefreshProgress?.completed).toBe(1);
    expect(store.currentRefreshProgress?.total).toBe(2);
  });
});
