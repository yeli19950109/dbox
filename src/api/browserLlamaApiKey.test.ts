import { beforeEach, expect, it, vi } from "vitest";
import { readBrowserLlamaApiKey, saveBrowserLlamaApiKey } from "./browserLlamaApiKey";
import { httpTransport } from "./httpTransport";
import { unwrapCommand } from "./transport";

const records = vi.hoisted(() => new Map<string, unknown>());
vi.mock("idb-keyval", () => ({
  createStore: vi.fn(),
  get: async (key: string) => structuredClone(records.get(key)),
  set: async (key: string, value: unknown) => { records.set(key, structuredClone(value)); },
  del: async (key: string) => { records.delete(key); },
}));

beforeEach(async () => {
  records.clear();
  const { webcrypto } = await vi.importActual<{ webcrypto: Crypto }>("node:crypto");
  vi.stubGlobal("crypto", webcrypto);
});

it("restores, replaces and clears the browser's encrypted API key", async () => {
  expect(await readBrowserLlamaApiKey()).toBeNull();
  await saveBrowserLlamaApiKey("first test key");
  const record = records.get("llama-server");
  expect(record).toMatchObject({ key: { extractable: false }, ciphertext: { byteLength: expect.any(Number) } });
  expect(await readBrowserLlamaApiKey()).toBe("first test key");
  await saveBrowserLlamaApiKey("second '测试' key");
  expect(await readBrowserLlamaApiKey()).toBe("second '测试' key");
  await saveBrowserLlamaApiKey("");
  expect(await readBrowserLlamaApiKey()).toBeNull();
  expect(records.size).toBe(0);
});

it("keeps the v0.3.1 default entry and isolates profile updates and removal", async () => {
  const first = "21964aa1-61be-4761-97fd-503fe21affb0";
  const second = "21964aa1-61be-4761-97fd-503fe21affb1";
  await saveBrowserLlamaApiKey("legacy");
  expect(await readBrowserLlamaApiKey(null)).toBe("legacy");
  expect(await readBrowserLlamaApiKey("default")).toBe("legacy");
  expect(await readBrowserLlamaApiKey(first)).toBeNull();
  await unwrapCommand(httpTransport.commands.llamaSaveApiKey("one", first));
  expect(await readBrowserLlamaApiKey(first)).toBe("one");
  await saveBrowserLlamaApiKey("two", second);
  await saveBrowserLlamaApiKey("updated", first);
  expect(await readBrowserLlamaApiKey(first.toUpperCase())).toBe("updated");
  await saveBrowserLlamaApiKey("", first);
  expect(await readBrowserLlamaApiKey(first)).toBeNull();
  expect(await unwrapCommand(httpTransport.commands.llamaReadApiKey(second))).toBe("two");
  expect(await readBrowserLlamaApiKey()).toBe("legacy");
  await expect(saveBrowserLlamaApiKey("", "some name")).rejects.toThrow("无效的配置 ID");
  await expect(readBrowserLlamaApiKey("")).rejects.toThrow("无效的配置 ID");
});
