import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTransportForTests, type ApiTransport } from "../api/transport";
import { apiError, createMockTransport, ok } from "../test/mockTransport";
import LlamaView from "./LlamaView.vue";

const root = "/Users/test/.llama-app";
function installed(tag: string) {
  const directory = `${root}/llama-${tag}`;
  return { tag, directory, serverPath: `${directory}/llama-server` };
}
const old = installed("b12644");
const current = installed("b12645");
const other = installed("b12646");
function button(wrapper: VueWrapper, text: string) {
  const match = wrapper.findAll("button").find(item => item.text() === text);
  if (!match) throw new Error(`Button missing: ${text}`);
  return match;
}
function deferred<T>() {
  let resolve!: (result: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

async function render(overrides: Partial<ApiTransport["commands"]> = {}) {
  const mock = createMockTransport({
    llamaInventory: vi.fn(() => ok({ root, versions: [current, old] })),
    ...overrides,
  });
  setTransportForTests(mock.transport);
  const wrapper = mount(LlamaView);
  await flushPromises();
  return { wrapper, commands: mock.transport.commands };
}

beforeEach(() => {
  setActivePinia(createPinia());
  window.localStorage.clear();
});

describe("llama.cpp", () => {
  it("uses the selected executable and native picker, masks API keys and copies quoted arguments", async () => {
    const model = "/Models/Wei's $(name).gguf";
    const chooseModel = vi.fn().mockImplementationOnce(() => ok(model)).mockImplementationOnce(() => ok(null));
    const { wrapper } = await render({ llamaChooseModel: chooseModel });
    expect(button(wrapper, "复制命令").attributes("disabled")).toBeDefined();
    await wrapper.get('input[aria-label="选择 b12644"]').setValue(true);
    await button(wrapper, "选择模型").trigger("click");
    await flushPromises();
    await button(wrapper, "更换模型").trigger("click");
    await flushPromises();
    expect(chooseModel).toHaveBeenCalledTimes(2);
    await wrapper.get('input[type="password"]').setValue("k '$HOME");
    await wrapper.get('input[inputmode="numeric"]').setValue("12345");
    await wrapper.findAll('input[inputmode="numeric"]')[1]!.setValue("4096");
    await button(wrapper, "全量（999）").trigger("click");
    expect(wrapper.get("pre").text()).toContain(old.serverPath);
    expect(wrapper.get("pre").text()).not.toContain("$HOME");
    await button(wrapper, "复制命令").trigger("click");
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      `${old.serverPath} -m '/Models/Wei'\\''s $(name).gguf' --host 127.0.0.1 --port 12345 --api-key 'k '\\''$HOME' --ctx-size 4096 --n-gpu-layers 999`,
    );
    expect(window.localStorage.getItem("dbox.llama.command")).not.toContain("$HOME");
    await wrapper.get('input[inputmode="numeric"]').setValue("65536");
    expect(wrapper.find("pre").exists()).toBe(false);
    expect(button(wrapper, "复制命令").attributes("disabled")).toBeDefined();
    wrapper.unmount();
  });

  it("installs a browsed release side by side and confirms only the version being removed", async () => {
    const { wrapper, commands } = await render({
      llamaReleases: vi.fn(({ page }) => ok({
        releases: [{ tag: page === 1 ? current.tag : other.tag, publishedAt: "2026-11-24", assetName: "archive.zip" }],
        hasMore: page === 1,
      })),
      llamaInstall: vi.fn(() => ok(other)),
      llamaRemove: vi.fn(() => ok(null)),
    });
    expect(commands.llamaReleases).not.toHaveBeenCalled();
    await button(wrapper, "下载版本").trigger("click");
    await flushPromises();
    expect(wrapper.get('button[aria-label="下载 b12645"]').attributes("disabled")).toBeDefined();
    await button(wrapper, "下一页").trigger("click");
    await flushPromises();
    await wrapper.get('button[aria-label="下载 b12646"]').trigger("click");
    await flushPromises();
    expect(commands.llamaInstall).toHaveBeenCalledWith({ tag: other.tag });
    await button(wrapper, "已安装 · 3").trigger("click");
    expect((wrapper.get('input[aria-label="选择 b12645"]').element as HTMLInputElement).checked).toBe(true);
    await wrapper.get('button[aria-label="移除 b12644"]').trigger("click");
    expect(wrapper.text()).toContain(old.directory);
    await button(wrapper, "取消").trigger("click");
    expect(commands.llamaRemove).not.toHaveBeenCalled();
    await wrapper.get('button[aria-label="移除 b12646"]').trigger("click");
    await button(wrapper, "确认移除").trigger("click");
    await flushPromises();
    expect(commands.llamaRemove).toHaveBeenCalledWith({ tag: other.tag });
    expect(wrapper.find('input[aria-label="选择 b12646"]').exists()).toBe(false);
    expect(wrapper.findAll('input[type="radio"]')).toHaveLength(2);
    wrapper.unmount();
  });

  it("restores the same saved model, options and key on switch and remount, and can clear the key", async () => {
    window.localStorage.setItem("dbox.llama.command", JSON.stringify({
      model: "/Models/existing.gguf", host: "localhost", port: "9310", context: "2048",
    }));
    const { wrapper, commands } = await render();
    expect(wrapper.text()).toContain("所有版本共用");
    const keyInput = wrapper.get("#llama-api-key");
    await keyInput.setValue("saved-key");
    await button(wrapper, "保存密钥").trigger("click");
    await flushPromises();
    expect(commands.llamaSaveApiKey).toHaveBeenCalledWith("saved-key");
    expect(wrapper.text()).toContain("密钥已保存。");
    await wrapper.get('input[aria-label="选择 b12644"]').setValue(true);
    expect(commands.llamaReadApiKey).toHaveBeenCalledTimes(1);
    expect(wrapper.get("pre").text()).toContain(old.serverPath);
    expect(wrapper.get("pre").text()).not.toContain("saved-key");
    wrapper.unmount();

    setActivePinia(createPinia());
    const reopened = mount(LlamaView);
    await flushPromises();
    expect(commands.llamaReadApiKey).toHaveBeenCalledTimes(2);
    expect((reopened.get("#llama-api-key").element as HTMLInputElement).value).toBe("saved-key");
    await button(reopened, "复制命令").trigger("click");
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      `${old.serverPath} -m /Models/existing.gguf --host localhost --port 9310 --api-key saved-key --ctx-size 2048`,
    );
    expect(window.localStorage.getItem("dbox.llama.command")).not.toContain("saved-key");
    await reopened.get("#llama-api-key").setValue("");
    await button(reopened, "清除已保存密钥").trigger("click");
    await flushPromises();
    expect(commands.llamaSaveApiKey).toHaveBeenLastCalledWith("");
    expect(reopened.text()).toContain("已清除保存的密钥。");
    expect(reopened.get("pre").text()).not.toContain("--api-key");
    reopened.unmount();
    const cleared = mount(LlamaView);
    await flushPromises();
    expect((cleared.get("#llama-api-key").element as HTMLInputElement).value).toBe("");
    expect(commands.llamaSaveApiKey).toHaveBeenCalledTimes(2);
    cleared.unmount();
  });

  it("preserves edits on a late read and waits for a pending rotation when returning to the page", async () => {
    const read = deferred<Awaited<ReturnType<typeof ok<string | null>>>>();
    const write = deferred<void>();
    let stored = "old-key";
    const { wrapper, commands } = await render({
      llamaReadApiKey: vi.fn().mockReturnValueOnce(read.promise).mockImplementation(() => ok(stored)),
      llamaSaveApiKey: vi.fn(async (value: string) => {
        await write.promise;
        stored = value;
        return ok(null);
      }),
    });
    await wrapper.get("#llama-api-key").setValue("new-key");
    expect(button(wrapper, "保存密钥").attributes("disabled")).toBeDefined();
    read.resolve({ status: "ok", data: stored });
    await flushPromises();
    expect((wrapper.get("#llama-api-key").element as HTMLInputElement).value).toBe("new-key");
    await button(wrapper, "保存密钥").trigger("click");
    expect(wrapper.text()).toContain("正在保存密钥");
    wrapper.unmount();
    const reopened = mount(LlamaView);
    await flushPromises();
    expect(commands.llamaReadApiKey).toHaveBeenCalledTimes(1);
    write.resolve();
    await flushPromises();
    expect((reopened.get("#llama-api-key").element as HTMLInputElement).value).toBe("new-key");
    expect(commands.llamaReadApiKey).toHaveBeenCalledTimes(2);
    reopened.unmount();
  });

  it("keeps copy opt-in after a failed key read and keeps the editable draft after a failed save", async () => {
    window.localStorage.setItem("dbox.llama.command", JSON.stringify({ model: "/Models/test.gguf" }));
    const { wrapper, commands } = await render({
      llamaReadApiKey: vi.fn(() => apiError("unavailable", "读取被拒绝")),
      llamaSaveApiKey: vi.fn().mockImplementationOnce(() => apiError("unavailable", "写入被拒绝")).mockImplementation(() => ok(null)),
    });
    expect(wrapper.text()).toContain("读取被拒绝");
    expect(button(wrapper, "复制命令").attributes("disabled")).toBeDefined();
    await button(wrapper, "本次不使用密钥").trigger("click");
    expect(wrapper.get("pre").text()).not.toContain("--api-key");
    expect(commands.llamaSaveApiKey).not.toHaveBeenCalled();
    await wrapper.get("#llama-api-key").setValue("draft-for-retry");
    await button(wrapper, "保存密钥").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("写入被拒绝");
    expect((wrapper.get("#llama-api-key").element as HTMLInputElement).value).toBe("draft-for-retry");
    expect(button(wrapper, "保存密钥").attributes("disabled")).toBeUndefined();
    await button(wrapper, "保存密钥").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("密钥已保存。");
    expect(commands.llamaSaveApiKey).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("offers an exact tag download when release browsing fails and reports install errors", async () => {
    const { wrapper, commands } = await render({
      llamaReleases: vi.fn(() => apiError("internal", "无法读取列表")),
      llamaInstall: vi.fn(() => apiError("internal", "构建不存在")),
    });
    await button(wrapper, "下载版本").trigger("click");
    await flushPromises();
    for (const invalidTag of ["../bad", "b4294967296", "b00000000001"]) {
      await wrapper.get("#llama-download-tag").setValue(invalidTag);
      expect(button(wrapper, "下载").attributes("disabled")).toBeDefined();
      await wrapper.get("form").trigger("submit");
    }
    expect(commands.llamaInstall).not.toHaveBeenCalled();
    await wrapper.get("#llama-download-tag").setValue("b99");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(commands.llamaInstall).toHaveBeenCalledWith({ tag: "b99" });
    expect(wrapper.text()).toContain("构建不存在");
    expect(wrapper.text()).toContain("已安装 · 2");
    wrapper.unmount();
  });
});
