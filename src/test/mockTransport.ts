import { vi } from "vitest";
import type { ApiTransport } from "../api/transport";
import type {
  ApiErrorDto,
  RefreshProgressEventDto,
  RunOutputEventDto,
  RunStateEventDto,
  ToolStateEventDto,
} from "../bindings";
import { settingsDocument, snapshotWithTools } from "./fixtures";

export function ok<T>(data: T): Promise<{ status: "ok"; data: T }> {
  return Promise.resolve({ status: "ok", data });
}

export function apiError(
  code: ApiErrorDto["code"],
  message: string,
): Promise<{ status: "error"; error: ApiErrorDto }> {
  return Promise.resolve({
    status: "error",
    error: { code, message, retryable: code !== "invalid_plan", details: {} },
  });
}

type EventPayloads = {
  refreshProgress: RefreshProgressEventDto;
  runOutput: RunOutputEventDto;
  runState: RunStateEventDto;
  toolState: ToolStateEventDto;
};

export function createMockTransport(
  commandOverrides: Partial<ApiTransport["commands"]> = {},
): {
  transport: ApiTransport;
  emit<K extends keyof EventPayloads>(name: K, payload: EventPayloads[K]): void;
} {
  const listeners = new Map<keyof EventPayloads, Array<(event: { payload: never }) => void>>();

  function channel(name: keyof EventPayloads): unknown {
    return {
      listen: vi.fn((callback: (event: { payload: never }) => void) => {
        const entries = listeners.get(name) ?? [];
        entries.push(callback);
        listeners.set(name, entries);
        return Promise.resolve(() => {
          listeners.set(name, (listeners.get(name) ?? []).filter((item) => item !== callback));
        });
      }),
      once: vi.fn(),
      emit: vi.fn(),
    };
  }

  const snapshot = snapshotWithTools();
  const baseCommands = {
    snapshot: vi.fn(() => ok(snapshot)),
    refresh: vi.fn(() => ok(snapshot)),
    preview: vi.fn(() => ok([])),
    confirm: vi.fn(),
    cancel: vi.fn((request) =>
      ok({ runId: request.runId, disposition: "cancellation_requested" as const }),
    ),
    runHistory: vi.fn(() => ok({ runs: [] })),
    runLog: vi.fn((request) => ok({ runId: request.runId, entries: [] })),
    settings: vi.fn(() => ok(settingsDocument())),
    saveSettings: vi.fn((request) =>
      ok({ revision: "settings-2", settings: request.settings }),
    ),
    validateManifest: vi.fn((request) =>
      ok({ fileName: request.fileName, manifestId: "fixture", normalizedToml: request.contents }),
    ),
    readManifest: vi.fn((request) =>
      ok({ fileName: request.fileName, revision: null, contents: null }),
    ),
    saveManifest: vi.fn(),
    ...commandOverrides,
  } as ApiTransport["commands"];

  const transport: ApiTransport = {
    commands: baseCommands,
    events: {
      refreshProgress: channel("refreshProgress"),
      runOutput: channel("runOutput"),
      runState: channel("runState"),
      toolState: channel("toolState"),
    } as ApiTransport["events"],
  };

  return {
    transport,
    emit(name, payload) {
      for (const callback of listeners.get(name) ?? []) {
        callback({ payload } as { payload: never });
      }
    },
  };
}
