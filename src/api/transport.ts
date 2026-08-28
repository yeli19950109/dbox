import { isTauri } from "@tauri-apps/api/core";
import { commands, events } from "../bindings";
import type { ApiErrorDto } from "../bindings";
import { httpTransport } from "./httpTransport";

export type ApiTransport = {
  commands: typeof commands;
  events: typeof events;
};

const tauriTransport: ApiTransport = { commands, events };
let activeTransport = defaultTransport();

export function defaultTransport(): ApiTransport {
  return isTauri() ? tauriTransport : httpTransport;
}

export class BackendError extends Error {
  readonly code: ApiErrorDto["code"];
  readonly retryable: boolean;
  readonly details: ApiErrorDto["details"];

  constructor(error: ApiErrorDto) {
    super(error.message);
    this.name = "BackendError";
    this.code = error.code;
    this.retryable = error.retryable;
    this.details = error.details;
  }
}

export function getTransport(): ApiTransport {
  return activeTransport;
}

export function setTransportForTests(transport: ApiTransport): void {
  activeTransport = transport;
}

export function resetTransport(): void {
  activeTransport = defaultTransport();
}

export function errorMessage(reason: unknown, fallback: string): string {
  if (reason instanceof BackendError) return reason.message;
  if (reason instanceof Error) {
    if (/invoke|__TAURI|TAURI_INTERNALS/i.test(reason.message)) return fallback;
    return reason.message;
  }
  return String(reason);
}

export async function unwrapCommand<T>(
  result: Promise<
    { status: "ok"; data: T } | { status: "error"; error: ApiErrorDto }
  >,
): Promise<T> {
  const response = await result;
  if (response.status === "error") {
    throw new BackendError(response.error);
  }
  return response.data;
}
