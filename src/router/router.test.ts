import { describe, expect, it } from "vitest";
import { router } from "./index";

describe("application routes", () => {
  it("uses Vue Router for current and future navigation", () => {
    const routes = router.getRoutes();
    expect(routes.map((route) => route.name)).toEqual(
      expect.arrayContaining(["tools", "runs", "confirm", "settings", "skills"]),
    );
    expect(routes.find((route) => route.name === "tools")?.path).toBe("/tools");
  });
});
