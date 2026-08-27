import { mount } from "@vue/test-utils";
import axe from "axe-core";
import { describe, expect, it } from "vitest";
import RunLog from "./RunLog.vue";

describe("RunLog", () => {
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
