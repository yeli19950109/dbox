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

function eventChannel<T>(eventName: string) {
  const listen = async (callback: BrowserEventCallback<T>): Promise<() => void> => {
    const source = new EventSource(`${HTTP_PREFIX}/events`);
    const handler = (event: Event) => {
      const message = event as MessageEvent<string>;
      try {
        callback({ event: eventName, id: 0, payload: JSON.parse(message.data) as T });
      } catch (reason) {
        console.error(`Ignored invalid ${eventName} Dev HTTP event`, reason);
      }
    };
    source.addEventListener(eventName, handler);
    return () => {
      source.removeEventListener(eventName, handler);
      source.close();
    };
  };

  const once = async (callback: BrowserEventCallback<T>): Promise<() => void> => {
    let cleanup = () => undefined;
    cleanup = await listen((event) => {
      cleanup();
      callback(event);
    });
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
  confirm: (request: Parameters<typeof generatedCommands.confirm>[0]) =>
    command<ConfirmResponseDto>("confirm", request),
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
