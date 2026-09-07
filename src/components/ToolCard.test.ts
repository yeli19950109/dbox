import { mount } from "@vue/test-utils";
import axe from "axe-core";
import { describe, expect, it } from "vitest";
import { installation, tool } from "../test/fixtures";
import ToolCard from "./ToolCard.vue";

describe("ToolCard", () => {
  it("distinguishes coexisting mise versions and their global activation", () => {
    for (const scope of ["global", "user"]) {
      const mise = installation(0, "mise");
      mise.scope = scope;
      mise.installedVersion = "22.1.0";
      const wrapper = mount(ToolCard, {
        props: {
          record: { tool: tool(0, "mise"), installations: [mise], providerIds: ["mise"] },
          selected: new Set<string>(),
        },
      });
      const label = `22.1.0 · ${scope === "global" ? "全局启用" : "未全局启用"}`;
      expect(wrapper.text()).toContain(label);
      expect(wrapper.get("button").attributes("aria-label")).toContain(label);
    }
  });

  it("renders the tool name, source and status in the default summary", () => {
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

    expect(wrapper.text()).toContain(unknownTool.displayName);
    expect(wrapper.get(".source-badge").text()).toBe("npm");
    expect(wrapper.text()).toContain("未知");
    expect(wrapper.text()).not.toContain("已是最新");
    expect(wrapper.text()).not.toContain("Core");
    expect(wrapper.text()).not.toContain("Component");
    expect(wrapper.text()).not.toContain("Executable");
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false);
  });

  it("uses readable source names for built-in providers", () => {
    for (const [id, label] of [["npm-global", "npm"], ["homebrew", "brew"], ["mise", "mise"]]) {
      const wrapper = mount(ToolCard, {
        props: {
          record: { tool: tool(1), installations: [], providerIds: [id!] },
          selected: new Set<string>(),
        },
      });
      expect(wrapper.get(".source-badge").text()).toBe(label);
    }
  });

  it("offers one bulk checkbox for direct updates when hasUpdates is null", async () => {
    const updateTool = tool(0, "npm");
    updateTool.status.hasUpdates = null;
    const wrapper = mount(ToolCard, {
      props: {
        record: {
          tool: updateTool,
          installations: [installation(0, "npm")],
          providerIds: ["npm"],
        },
        selected: new Set<string>(),
      },
    });

    const checkbox = wrapper.get('input[type="checkbox"]');
    expect(checkbox.attributes("aria-label")).toContain("全部更新");
    expect(checkbox.attributes("disabled")).toBeUndefined();
    await checkbox.setValue(true);
    expect(wrapper.emitted("select-updates")).toHaveLength(1);
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
