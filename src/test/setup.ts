import { afterEach, vi } from "vitest";
import { config } from "@vue/test-utils";
import { resetTransport } from "../api/transport";

class ResizeObserverStub implements ResizeObserver {
  readonly root = null;
  readonly rootMargin = "0px";
  readonly thresholds = [];
  private readonly callback: ResizeObserverCallback;

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }

  disconnect(): void {}
  observe(target: Element): void {
    // TanStack can use the fixed estimate for rows; only the scroll container
    // needs a deterministic test rect. Real browsers report row resize async.
    if (target.classList.contains("virtual-list__row")) return;
    const contentRect = target.getBoundingClientRect();
    this.callback(
      [
        {
          target,
          contentRect,
          borderBoxSize: [{ inlineSize: contentRect.width, blockSize: contentRect.height }],
          contentBoxSize: [{ inlineSize: contentRect.width, blockSize: contentRect.height }],
          devicePixelContentBoxSize: [],
        } as unknown as ResizeObserverEntry,
      ],
      this,
    );
  }
  unobserve(): void {}
  takeRecords(): ResizeObserverEntry[] {
    return [];
  }
}

vi.stubGlobal("ResizeObserver", ResizeObserverStub);
vi.stubGlobal("scrollTo", vi.fn());
vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function () {
  const height = this.classList.contains("log-viewport")
    ? 520
    : this.classList.contains("tool-virtual-list")
      ? 720
      : this.classList.contains("virtual-list__row")
        ? 245
        : 40;
  return {
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    right: 900,
    bottom: height,
    width: 900,
    height,
    toJSON: () => ({}),
  };
});
vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
  () => ({ measureText: () => ({ width: 0 }) }) as unknown as CanvasRenderingContext2D,
);
Object.defineProperty(navigator, "clipboard", {
  configurable: true,
  value: { writeText: vi.fn().mockResolvedValue(undefined) },
});

config.global.stubs = {
  transition: false,
};

afterEach(() => {
  resetTransport();
  vi.clearAllMocks();
  vi.useRealTimers();
});
