import { vi } from "vitest";
import type { ApiTransport } from "../api/transport";
import type {
  ApiErrorDto,
  ExtensionChangedEventDto,
  McpChangedEventDto,
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
  skillsChanged: ExtensionChangedEventDto;
  mcpChanged: McpChangedEventDto;
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
    confirmBatch: vi.fn(),
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
    listAgentTargets: vi.fn(() => ok([])),
    saveAgentTargets: vi.fn(() => ok([])),
    listSkills: vi.fn(() => ok({ revision: "missing", skills: [], sources: [] })),
    listSkillSources: vi.fn(() => ok({ revision: "missing", skills: [], sources: [] })),
    listSkillBackups: vi.fn(() => ok([])),
    listMcpServers: vi.fn(() => ok({ revision: "missing", servers: [] })),
    scanSkillImports: vi.fn(() => ok({ candidates: [], errors: [], checkedAt: "" })),
    scanMcpImports: vi.fn(() => ok({ candidates: [], errors: [] })),
    codexUsage: vi.fn((request) =>
      ok({
        codexHome: "/home/test/.codex",
        ledgerDir: "/home/test/data/usage",
        available: false,
        generatedAt: "2026-07-10T12:00:00Z",
        days: request.days,
        filesScanned: 0,
        sessions: 0,
        totals: { requests: 0, inputTokens: 0, cachedInputTokens: 0, cacheWriteInputTokens: 0, outputTokens: 0, totalTokens: 0 },
        models: [],
        daily: [],
        deferred: [],
        replayedRequests: 0,
        duplicateRequests: 0,
      }),
    ),
    ...commandOverrides,
  } as ApiTransport["commands"];

  const transport: ApiTransport = {
    commands: baseCommands,
    events: {
      skillsChanged: channel("skillsChanged"),
      mcpChanged: channel("mcpChanged"),
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
