import { createPinia, setActivePinia } from "pinia";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setTransportForTests } from "../api/transport";
import { apiError, createMockTransport } from "../test/mockTransport";
import SettingsView from "./SettingsView.vue";

describe("SettingsView revision conflicts", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("keeps the user's settings draft when the backend reports a conflict", async () => {
    const mock = createMockTransport({
      saveSettings: vi.fn(() => apiError("conflict", "revision changed")),
    });
    setTransportForTests(mock.transport);
    const wrapper = mount(SettingsView, { attachTo: document.body });
    await flushPromises();

    const numeric = wrapper.findAll('input[inputmode="numeric"]')[0]!;
    await numeric.setValue("333");
    const submit = wrapper.get('[data-testid="settings-save"]');
    expect(submit.attributes("disabled")).toBeUndefined();
    await submit.trigger("click");
    await flushPromises();

    expect(mock.transport.commands.saveSettings).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain("设置已在其他位置发生变化");
    expect((numeric.element as HTMLInputElement).value).toBe("333");
    wrapper.unmount();
  });

  it("keeps manifest contents and advances the remote revision after conflict", async () => {
    const saveManifest = vi.fn(() => apiError("conflict", "manifest changed"));
    const mock = createMockTransport({
      saveManifest,
      readManifest: vi.fn((request) =>
        Promise.resolve({
          status: "ok" as const,
          data: { fileName: request.fileName, revision: "remote-2", contents: "remote = true" },
        }),
      ),
    });
    setTransportForTests(mock.transport);
    const wrapper = mount(SettingsView, { attachTo: document.body });
    await flushPromises();
    const textarea = wrapper.get("textarea");
    await textarea.setValue('id = "local"');
    const save = wrapper.findAll("button").find((button) => button.text() === "校验并保存")!;
    await save.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("Manifest revision 冲突");
    expect((textarea.element as HTMLTextAreaElement).value).toBe('id = "local"');
    expect(wrapper.text()).toContain("remote-2");
    wrapper.unmount();
  });
});
