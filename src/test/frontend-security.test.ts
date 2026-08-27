import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

describe("frontend execution boundary", () => {
  it("contains no process or shell execution entry point outside generated transport", () => {
    const sourceRoot = join(process.cwd(), "src");
    const applicationFiles = (readdirSync(sourceRoot, { recursive: true }) as string[])
      .filter((file) => /\.(ts|vue)$/.test(file))
      .filter((file) => !file.includes(".test.") && !file.startsWith("test/") && file !== "bindings.ts");
    const forbidden = [
      "@tauri-apps/plugin-shell",
      "node:child_process",
      "Deno.Command",
      "Bun.spawn",
    ];

    for (const file of applicationFiles) {
      const contents = readFileSync(join(sourceRoot, file), "utf8");
      for (const token of forbidden) {
        expect(contents, `${file} must not contain ${token}`).not.toContain(token);
      }
    }
  });
});
