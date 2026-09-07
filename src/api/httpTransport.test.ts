import { commands as generatedCommands } from "../bindings";
import { httpTransport } from "./httpTransport";
import {
  BackendError,
  defaultTransport,
  getTransport,
  resetTransport,
  unwrapCommand,
} from "./transport";

type MockResponse = Pick<Response, "ok" | "status" | "text">;

function response(body: unknown, status = 200): MockResponse {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: vi.fn().mockResolvedValue(
      typeof body === "string" ? body : JSON.stringify(body),
    ),
  };
}

class EventSourceMock {
  static readonly OPEN = 1;
  readyState = 0;
  readonly url: string;
  readonly listeners = new Map<string, Set<EventListener>>();
  close = vi.fn();
  removeEventListener = vi.fn((name: string, listener: EventListener) => {
    this.listeners.get(name)?.delete(listener);
  });

  constructor(url: string | URL) {
    this.url = String(url);
    eventSources.push(this);
  }

  addEventListener(name: string, listener: EventListener): void {
    const listeners = this.listeners.get(name) ?? new Set<EventListener>();
    listeners.add(listener);
    this.listeners.set(name, listeners);
  }

  dispatch(name: string, payload: unknown): void {
    if (name === "open") this.readyState = 1;
    const event = new MessageEvent(name, { data: JSON.stringify(payload) });
    for (const listener of this.listeners.get(name) ?? []) listener(event);
  }
}

const eventSources: EventSourceMock[] = [];
const originalFetch = globalThis.fetch;
const originalEventSource = globalThis.EventSource;

afterEach(() => {
  eventSources.splice(0);
  Object.defineProperty(globalThis, "fetch", {
    configurable: true,
    writable: true,
    value: originalFetch,
  });
  Object.defineProperty(globalThis, "EventSource", {
    configurable: true,
    writable: true,
    value: originalEventSource,
  });
  vi.stubGlobal("isTauri", false);
});

describe("Dev HTTP commands", () => {
  it("posts typed requests and returns successful envelopes", async () => {
    const snapshot = { schemaRevision: 1 };
    const fetchMock = vi
      .fn()
      .mockResolvedValue(response({ status: "ok", data: snapshot }));
    vi.stubGlobal("fetch", fetchMock);

    const result = await httpTransport.commands.refresh({
      scope: { scope: "provider", providerId: "npm" },
      force: true,
    });

    expect(result).toEqual({ status: "ok", data: snapshot });
    expect(fetchMock).toHaveBeenCalledWith(
      "/__dbox_http/commands/refresh",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          scope: { scope: "provider", providerId: "npm" },
          force: true,
        }),
      }),
    );
  });

  it("preserves business errors for unwrapCommand", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        response({
          status: "error",
          error: {
            code: "conflict",
            message: "revision changed",
            retryable: true,
            details: { field: "expectedRevision" },
          },
        }),
      ),
    );

    await expect(unwrapCommand(httpTransport.commands.settings())).rejects.toMatchObject({
      name: "BackendError",
      code: "conflict",
      retryable: true,
      details: { field: "expectedRevision" },
    } satisfies Partial<BackendError>);
  });

  it("reports invalid JSON and network failures", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(response("not-json"))
      .mockRejectedValueOnce(new TypeError("fetch failed"));
    vi.stubGlobal("fetch", fetchMock);

    await expect(httpTransport.commands.snapshot()).rejects.toThrow(
      "returned invalid JSON",
    );
    await expect(httpTransport.commands.snapshot()).rejects.toThrow("fetch failed");
  });

  it("reports non-success HTTP responses before parsing envelopes", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response("bad request", 422)));

    await expect(httpTransport.commands.refresh({
      scope: { scope: "all" },
      force: true,
    })).rejects.toThrow("HTTP 422: bad request");
  });
});

describe("Dev HTTP events", () => {
  it("shares a stream and waits for it to open before confirming a command", async () => {
    vi.stubGlobal("EventSource", EventSourceMock);
    const fetchMock = vi.fn().mockResolvedValue(response({ status: "ok", data: {} }));
    vi.stubGlobal("fetch", fetchMock);
    const stopOutput = await httpTransport.events.runOutput.listen(vi.fn());
    const stopState = await httpTransport.events.runState.listen(vi.fn());
    expect(eventSources).toHaveLength(1);
    const pending = httpTransport.commands.confirm({ planId: "plan", planHash: "hash" });
    expect(fetchMock).not.toHaveBeenCalled();
    eventSources[0]!.dispatch("open", {});
    await pending;
    expect(fetchMock).toHaveBeenCalledTimes(1);
    stopOutput();
    expect(eventSources[0]!.close).not.toHaveBeenCalled();
    stopState();
    expect(eventSources[0]!.close).toHaveBeenCalledTimes(1);
  });

  it("does not execute when the live stream cannot connect", async () => {
    vi.stubGlobal("EventSource", EventSourceMock);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const stop = await httpTransport.events.runOutput.listen(vi.fn());
    const pending = httpTransport.commands.confirm({ planId: "plan", planHash: "hash" });
    const rejected = expect(pending).rejects.toThrow("实时事件连接失败");
    eventSources[0]!.dispatch("error", {});
    await rejected;
    expect(fetchMock).not.toHaveBeenCalled();
    stop();
  });

  it("adapts SSE payloads and closes the connection when unlistened", async () => {
    vi.stubGlobal("EventSource", EventSourceMock);
    const listener = vi.fn();

    const unlisten = await httpTransport.events.toolState.listen(listener);
    expect(eventSources).toHaveLength(1);
    expect(eventSources[0].url).toBe("/__dbox_http/events");

    eventSources[0].dispatch("tool-state", {
      sequence: "7",
      stateRevision: "state-7",
      toolIds: ["tool"],
    });
    expect(listener).toHaveBeenCalledWith({
      event: "tool-state",
      id: 0,
      payload: {
        sequence: "7",
        stateRevision: "state-7",
        toolIds: ["tool"],
      },
    });

    unlisten();
    expect(eventSources[0].removeEventListener).toHaveBeenCalled();
    expect(eventSources[0].close).toHaveBeenCalled();
  });
});

describe("automatic transport selection", () => {
  it("uses HTTP in a regular browser and generated bindings in Tauri", () => {
    vi.stubGlobal("isTauri", false);
    expect(defaultTransport()).toBe(httpTransport);
    resetTransport();
    expect(getTransport()).toBe(httpTransport);

    vi.stubGlobal("isTauri", true);
    const tauri = defaultTransport();
    expect(tauri).not.toBe(httpTransport);
    expect(tauri.commands).toBe(generatedCommands);

    vi.stubGlobal("isTauri", false);
  });
});
