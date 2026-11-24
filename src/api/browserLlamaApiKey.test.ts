import { expect, it, vi } from "vitest";
import { readBrowserLlamaApiKey, saveBrowserLlamaApiKey } from "./browserLlamaApiKey";

const records = vi.hoisted(() => new Map<string, unknown>());
vi.mock("idb-keyval", () => ({
  createStore: vi.fn(),
  get: async (key: string) => structuredClone(records.get(key)),
  set: async (key: string, value: unknown) => { records.set(key, structuredClone(value)); },
  del: async (key: string) => { records.delete(key); },
}));

it("restores, replaces and clears the browser's encrypted API key", async () => {
  const { webcrypto } = await vi.importActual<{ webcrypto: Crypto }>("node:crypto");
  vi.stubGlobal("crypto", webcrypto);
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
