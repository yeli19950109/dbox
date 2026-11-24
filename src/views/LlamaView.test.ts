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
