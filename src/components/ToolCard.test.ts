import { mount } from "@vue/test-utils";
import axe from "axe-core";
import { describe, expect, it } from "vitest";
import { installation, tool } from "../test/fixtures";
import ToolCard from "./ToolCard.vue";

describe("ToolCard", () => {
  it("does not label unsupported or unknown versions as current", () => {
    const unknownTool = tool(5, "npm");
    const wrapper = mount(ToolCard, {
      props: {
        record: {
          tool: unknownTool,
          installations: [installation(5, "npm")],
          providerIds: ["npm"],
        },
        selected: new Set<string>(),
      },
    });

    expect(wrapper.text()).toContain("尚未确认");
    expect(wrapper.text()).toContain("未知");
    expect(wrapper.text()).not.toContain("已是最新");
    expect(wrapper.get('input[type="checkbox"]').attributes("disabled")).toBeDefined();
  });

  it("has accessible names and passes axe including the color-contrast rule", async () => {
    const record = {
      tool: tool(0, "npm"),
      installations: [installation(0, "npm")],
      providerIds: ["npm"],
    };
    const wrapper = mount(ToolCard, {
      attachTo: document.body,
      props: { record, selected: new Set<string>() },
    });

    const results = await axe.run(wrapper.element, {
      rules: { "color-contrast": { enabled: true } },
    });
    expect(wrapper.get("button").attributes("aria-label")).toContain("详情");
    expect(results.violations).toEqual([]);
    wrapper.unmount();
  });
});
