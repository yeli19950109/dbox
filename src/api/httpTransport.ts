import type {
  ApiErrorDto,
  CancelResponseDto,
  ConfirmResponseDto,
  ManifestDocumentDto,
  ManifestValidationDto,
  RefreshProgressEventDto,
  RunHistoryDto,
  RunLogDto,
  RunOutputEventDto,
  RunStateEventDto,
  SavedManifestDto,
  SettingsDocumentDto,
  SnapshotDto,
  ToolStateEventDto,
  UpdatePlanDto,
} from "../bindings";
import { commands as generatedCommands, events as generatedEvents } from "../bindings";
import type { ApiTransport } from "./transport";

const HTTP_PREFIX = "/__dbox_http";

type CommandResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: ApiErrorDto };

type BrowserEvent<T> = {
  event: string;
  id: number;
  payload: T;
};

type BrowserEventCallback<T> = (event: BrowserEvent<T>) => void;

async function command<T>(name: string, request?: unknown): Promise<CommandResult<T>> {
  const response = await fetch(`${HTTP_PREFIX}/commands/${name}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: request === undefined ? "{}" : JSON.stringify(request),
  });
  const body = await response.text();
  if (!response.ok) {
    const detail = body.trim();
    throw new Error(
      `Dev HTTP command ${name} failed with HTTP ${response.status}${detail ? `: ${detail}` : ""}`,
    );
  }

  let payload: unknown;
  try {
    payload = JSON.parse(body);
  } catch (reason) {
    throw new Error(`Dev HTTP command ${name} returned invalid JSON`, { cause: reason });
  }
  if (!isCommandResult<T>(payload)) {
    throw new Error(`Dev HTTP command ${name} returned an invalid response envelope`);
  }
  return payload;
}

function isCommandResult<T>(value: unknown): value is CommandResult<T> {
  if (typeof value !== "object" || value === null || !("status" in value)) return false;
  if (value.status === "ok") return "data" in value;
  if (value.status !== "error" || !("error" in value)) return false;
  const error = value.error;
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    "message" in error &&
    "retryable" in error &&
    "details" in error
  );
}

// All channels share one stream so output/state ordering is retained and long-lived
// SSE requests do not exhaust the browser's per-origin HTTP connection pool.
let eventStream: { source: EventSource; subscribers: number } | null = null;

function waitForOpen(source: EventSource): Promise<void> {
  if (source.readyState === EventSource.OPEN) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(timeout);
      source.removeEventListener("open", opened);
      source.removeEventListener("error", failed);
    };
    const opened = () => { cleanup(); resolve(); };
    const failed = () => { cleanup(); reject(new Error("实时事件连接失败")); };
    const timeout = setTimeout(() => { cleanup(); reject(new Error("实时事件连接超时")); }, 10_000);
    source.addEventListener("open", opened);
    source.addEventListener("error", failed);
  });
}

function eventChannel<T>(eventName: string) {
  const listen = async (callback: BrowserEventCallback<T>): Promise<() => void> => {
    const stream = eventStream ??= {
      source: new EventSource(`${HTTP_PREFIX}/events`),
      subscribers: 0,
    };
    stream.subscribers += 1;
    const handler = (event: Event) => {
      const message = event as MessageEvent<string>;
      try {
        callback({ event: eventName, id: 0, payload: JSON.parse(message.data) as T });
      } catch (reason) {
        console.error(`Ignored invalid ${eventName} Dev HTTP event`, reason);
      }
    };
    stream.source.addEventListener(eventName, handler);
    let stopped = false;
    const cleanup = () => {
      if (stopped) return;
      stopped = true;
      stream.source.removeEventListener(eventName, handler);
      if (--stream.subscribers === 0) {
        stream.source.close();
        if (eventStream === stream) eventStream = null;
      }
    };
    return cleanup;
  };

  const once = async (callback: BrowserEventCallback<T>): Promise<() => void> => {
    let cleanup: (() => void) | undefined;
    let received = false;
    cleanup = await listen((event) => {
      if (received) return;
      received = true;
      cleanup?.();
      callback(event);
    });
    if (received) cleanup();
    return cleanup;
  };

  const unsupportedEmit = async (_payload: T): Promise<void> => {
    throw new Error("Dev HTTP browser events are receive-only");
  };

  const channel = (_target: unknown) => channel;
  return Object.assign(channel, { listen, once, emit: unsupportedEmit });
}

const httpCommands = {
  snapshot: () => command<SnapshotDto>("snapshot"),
  refresh: (request: Parameters<typeof generatedCommands.refresh>[0]) =>
    command<SnapshotDto>("refresh", request),
  preview: (request: Parameters<typeof generatedCommands.preview>[0]) =>
    command<UpdatePlanDto[]>("preview", request),
  confirm: async (request: Parameters<typeof generatedCommands.confirm>[0]) => {
    // Keep subscriptions alive through reconnects, but do not start a command before
    // the stream carrying its first state/output events is actually open.
    if (eventStream) await waitForOpen(eventStream.source);
    return command<ConfirmResponseDto>("confirm", request);
  },
  cancel: (request: Parameters<typeof generatedCommands.cancel>[0]) =>
    command<CancelResponseDto>("cancel", request),
  runHistory: () => command<RunHistoryDto>("run_history"),
  runLog: (request: Parameters<typeof generatedCommands.runLog>[0]) =>
    command<RunLogDto>("run_log", request),
  settings: () => command<SettingsDocumentDto>("settings"),
  saveSettings: (request: Parameters<typeof generatedCommands.saveSettings>[0]) =>
    command<SettingsDocumentDto>("save_settings", request),
  validateManifest: (request: Parameters<typeof generatedCommands.validateManifest>[0]) =>
    command<ManifestValidationDto>("validate_manifest", request),
  readManifest: (request: Parameters<typeof generatedCommands.readManifest>[0]) =>
    command<ManifestDocumentDto>("read_manifest", request),
  saveManifest: (request: Parameters<typeof generatedCommands.saveManifest>[0]) =>
    command<SavedManifestDto>("save_manifest", request),
} satisfies typeof generatedCommands;

const httpEvents = {
  refreshProgress: eventChannel<RefreshProgressEventDto>("refresh-progress"),
  runOutput: eventChannel<RunOutputEventDto>("run-output"),
  runState: eventChannel<RunStateEventDto>("run-state"),
  toolState: eventChannel<ToolStateEventDto>("tool-state"),
} as typeof generatedEvents;

export const httpTransport: ApiTransport = {
  commands: httpCommands,
  events: httpEvents,
};
