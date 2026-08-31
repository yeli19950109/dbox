import { describe, expect, it } from "vitest";
import type { SnapshotDto } from "../bindings";
import { snapshotWithTools } from "../test/fixtures";
import {
  mapToolRecords,
  toolHasUpdates,
  updateableComponents,
} from "./presentation";

describe("generated DTO presentation mapping", () => {
  it("joins installations by generated ids and keeps equal names from different providers separate", () => {
    const snapshot = snapshotWithTools(10);
    const records = mapToolRecords(snapshot);
    const shared = records.filter((record) => record.tool.displayName === "Shared CLI");

    expect(shared).toHaveLength(2);
    expect(shared[0]?.providerIds).not.toEqual(shared[1]?.providerIds);
    expect(records[1]?.installations[0]?.providerId).toBe("brew");
  });

  it("ignores unknown backend fields and never treats unknown as updateable", () => {
    const raw = {
      ...snapshotWithTools(6),
      futureBackendField: { value: true },
    } as SnapshotDto;
    const records = mapToolRecords(raw);
    const unknown = records.find((record) => record.tool.status.status === "unknown");

    expect(records).toHaveLength(6);
    expect(unknown).toBeDefined();
    expect(updateableComponents(unknown!.tool)).toEqual([]);
  });

  it("recognizes a direct update_available status when aggregate metadata is null", () => {
    const update = snapshotWithTools(1).tools[0]!;
    update.status.hasUpdates = null;

    expect(update.status.status).toBe("update_available");
    expect(toolHasUpdates(update)).toBe(true);
  });
});
