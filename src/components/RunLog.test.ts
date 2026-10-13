import { flushPromises, mount } from "@vue/test-utils";
import axe from "axe-core";
import { describe, expect, it, vi } from "vitest";
import RunLog from "./RunLog.vue";

describe("RunLog", () => {
  it("updates mise progress in place and can reveal every original output chunk", async () => {
    const line = (sequence: string, message: string) => ({
      sequence, message, stream: "stderr", timestamp: "2026-10-13T06:01:16Z",
    });
    const first = line("8", "mise ░░░░░░░░░░░░░░░░ 0/1 · 3.0s\n  python@3.14.8  downloading  3.0s\n");
    const second = line("9", "mise ███░░░░░░░░░░░░░ 0/1 · 6.0s\n  python@3.14.8  downloading  6.0s  2.1/26.7 MB · 2.3 MB/s\n");
    const third = line("10", "mise ███████████░░░░░ 0/1 · 21.0s\n  python@3.14.8  verifying  21.0s  26.7/26.7 MB\n");
    const wrapper = mount(RunLog, {
      props: { lines: [first, second], revision: 1, status: "running" },
    });
    await flushPromises();
    expect(wrapper.findAll(".log-line")).toHaveLength(1);
    expect(wrapper.get("pre").text()).toBe(second.message.trim());
    expect(wrapper.get(".log-line__sequence").text()).toBe("8");
    const progressElement = wrapper.get(".log-line").element;
    await wrapper.setProps({ lines: [first, second, third], revision: 2 });
    await flushPromises();
    expect(wrapper.findAll(".log-line")).toHaveLength(1);
    expect(wrapper.get(".log-line").element).toBe(progressElement);
    expect(wrapper.get("pre").text()).toBe(third.message.trim());
    expect(wrapper.get(".log-line__stream").attributes("title")).toContain("不代表运行失败");
    expect(wrapper.get("button").text()).toBe("展开进度");

    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(wrapper.get("button").attributes("aria-expanded")).toBe("true");
    expect(wrapper.findAll("pre").map((pre) => pre.text())).toEqual([first, second, third].map((entry) => entry.message.trim()));
    expect(wrapper.get("button").text()).toBe("合并进度");
    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".log-line")).toHaveLength(1);

    const complete = line("11", "mise ████████████████ 1/1 · installed 1 tool in 24.9s\n");
    await wrapper.setProps({ lines: [first, second, third, complete], revision: 3, status: "succeeded" });
    await flushPromises();
    expect(wrapper.findAll("pre").map((pre) => pre.text())).toEqual([third.message.trim(), complete.message.trim()]);
    wrapper.unmount();
  });

  it("explains a silent run and replaces the waiting message with live output", async () => {
    const wrapper = mount(RunLog, {
      props: { lines: [], revision: 0, status: "queued" },
    });
    expect(wrapper.get('[role="log"]').text()).toContain("正在排队");
    await wrapper.setProps({ status: "running" });
    expect(wrapper.get('[role="log"]').text()).toContain("等待输出");
    await wrapper.setProps({
      lines: [{ sequence: "2", timestamp: "2026-09-11T05:25:26Z", stream: "system", message: "执行命令：npm install --global --loglevel=info -- @scope/tool@latest" }],
      revision: 1,
    });
    expect(wrapper.find(".log-empty").exists()).toBe(false);
    expect(wrapper.get('[role="log"]').text()).toContain("执行命令：npm install");
    wrapper.unmount();
  });

  it("positions subsequent chunks below the measured multiline output", async () => {
    const measure = vi.mocked(HTMLElement.prototype.getBoundingClientRect);
    const original = measure.getMockImplementation()!;
    measure.mockImplementation(function () {
      const rect = original.call(this);
      if (!this.classList.contains("log-line")) return rect;
      const height = (this.querySelector("pre")?.textContent?.split("\n").length ?? 1) * 18 + 10;
      return { ...rect, height, bottom: height };
    });
    const wrapper = mount(RunLog, {
      attachTo: document.body,
      props: {
        revision: 1,
        lines: [
          { sequence: "2", timestamp: "2026-09-11T05:25:26Z", stream: "stderr", message: "npm info using npm\nnpm http fetch GET 200 registry\nnpm info run postinstall" },
          { sequence: "3", timestamp: "2026-09-11T05:25:27Z", stream: "stdout", message: "updated" },
        ],
      },
    });
    try {
      await flushPromises();
      const rows = wrapper.findAll(".log-line");
      expect(rows).toHaveLength(2);
      expect(rows[1]!.attributes("style")).toContain("translateY(64px)");
      expect(wrapper.get(".virtual-list__sizer").attributes("style")).toContain("height: 92px");
    } finally {
      wrapper.unmount();
      measure.mockImplementation(original);
    }
  });

  it("keeps a ten-thousand-line log DOM bounded and exposes an accessible live log", async () => {
    const lines = Array.from({ length: 10_000 }, (_, index) => ({
      sequence: String(index + 1),
      timestamp: "2026-08-27T10:00:00Z",
      stream: index % 8 === 0 ? "stderr" : "stdout",
      message: `fixture line ${index}`,
    }));
    const wrapper = mount(RunLog, {
      attachTo: document.body,
      props: { lines, revision: 1 },
    });

    const rendered = wrapper.findAll(".log-line");
    expect(rendered.length).toBeGreaterThan(0);
    expect(rendered.length).toBeLessThan(80);
    expect(wrapper.get('[role="log"]').attributes("aria-live")).toBe("polite");

    const results = await axe.run(wrapper.element, {
      rules: { "color-contrast": { enabled: true } },
    });
    expect(results.violations).toEqual([]);
    wrapper.unmount();
  });
});
