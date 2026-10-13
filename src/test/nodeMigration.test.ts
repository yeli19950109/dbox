import { afterEach, describe, expect, it } from "vitest";
import { copyFileSync, existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";

const script = readFileSync(join(process.cwd(), "src-tauri/src/providers/node_migration.cjs"), "utf8");
const directories: string[] = [];
afterEach(() => directories.splice(0).forEach(path => rmSync(path, { recursive: true, force: true })));

function fixture() {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "dbox-node-migration-")));
  directories.push(base);
  const old = join(base, "old node"), next = join(base, "new node"), manifest = join(base, "state", "migration.json");
  for (const prefix of [old, next]) {
    mkdirSync(join(prefix, "bin"), { recursive: true });
    try { linkSync(process.execPath, join(prefix, "bin/node")); }
    catch { copyFileSync(process.execPath, join(prefix, "bin/node")); }
    mkdirSync(join(prefix, "lib/node_modules/npm/bin"), { recursive: true });
    writeFileSync(join(prefix, "fixture.json"), JSON.stringify({ dependencies: {}, allow: "", ignore: false }));
    writeFileSync(join(prefix, "lib/node_modules/npm/bin/npm-cli.js"), `
      const fs = require('node:fs'), path = require('node:path');
      const args = process.argv.slice(2), prefix = args[args.indexOf('--prefix') + 1];
      const config = JSON.parse(fs.readFileSync(path.join(prefix, 'fixture.json')));
      fs.appendFileSync(path.join(prefix, 'calls.jsonl'), JSON.stringify(args) + '\\n');
      if (args[0] === 'ls') {
        if (config.listFail) process.exit(1);
        console.log(JSON.stringify({ dependencies: config.dependencies }));
      } else if (args[0] === 'config') {
        console.log(JSON.stringify(args[2] === 'allow-scripts' ? config.allow : config.ignore));
      } else if (args[0] === 'install') {
        const spec = args[args.indexOf('--') + 1];
        if (spec === config.fail) process.exit(7);
        fs.appendFileSync(path.join(prefix, 'installed.txt'), spec + '\\n');
      } else process.exit(64);
    `);
  }
  function inventory(names: string[], allow: string | string[] = "", extra = {}) {
    names.forEach(name => mkdirSync(join(old, "lib/node_modules", name), { recursive: true }));
    writeFileSync(join(old, "fixture.json"), JSON.stringify({ dependencies: Object.fromEntries(names.map(name => [name, { version: "1.0.0" }])), allow, ignore: false, ...extra }));
  }
  function run(mode: string, policies = {}, runtime = mode === "snapshot" ? old : next) {
    return spawnSync(join(runtime, "bin/node"), ["-e", script, "--", mode, manifest, old, JSON.stringify(policies)], { encoding: "utf8" });
  }
  const calls = (prefix: string): string[][] => readFileSync(join(prefix, "calls.jsonl"), "utf8").trim().split("\n").map(line => JSON.parse(line));
  return { base, old, next, manifest, inventory, run, calls };
}

describe("global Node migration", () => {
  it("snapshots the old runtime and restores latest packages into the new runtime with recorded approvals", () => {
    const f = fixture();
    f.inventory(["npm", "@deepseek-ai/dsh", "pi"], ["protobufjs", "koffi"]);
    expect(f.run("snapshot", { "@deepseek-ai/dsh": ["node-pty", "koffi"] }).status).toBe(0);
    expect(f.calls(f.old).some(args => args[0] === "install")).toBe(false);
    expect(f.run("restore").status).toBe(0);
    const installs = f.calls(f.next).filter(args => args[0] === "install");
    expect(installs.map(args => args.at(-1))).toEqual(["npm@latest", "@deepseek-ai/dsh@latest", "pi@latest"]);
    expect(installs[1]).toContain("--allow-scripts=protobufjs,koffi,node-pty");
    expect(installs[2]).toContain("--allow-scripts=protobufjs,koffi");
    expect(installs.every(args => args.includes("--strict-allow-scripts") && args[args.indexOf("--prefix") + 1] === f.next)).toBe(true);
    const saved = JSON.parse(readFileSync(f.manifest, "utf8"));
    expect(saved.previousPrefix).toBe(f.old);
    expect(saved.results.every((item: { status: string }) => item.status === "succeeded")).toBe(true);
  });

  it("does not infer historical one-off approvals or enable all scripts", () => {
    const f = fixture(); f.inventory(["dsh"]);
    expect(f.run("snapshot").status).toBe(0);
    expect(f.run("restore").status).toBe(0);
    const args = f.calls(f.next)[0]!;
    expect(args.some(arg => arg.startsWith("--allow-scripts") || arg.includes("dangerously"))).toBe(false);
  });

  it("stops before update when inventory fails and refuses the wrong source runtime", () => {
    const f = fixture(); f.inventory(["pi"], "", { listFail: true });
    expect(f.run("snapshot").status).toBe(1);
    expect(existsSync(f.manifest)).toBe(false);
    expect(f.run("snapshot", {}, f.next).status).toBe(1);
  });

  it("retains a recovery manifest and continues remaining packages after an install failure", () => {
    const f = fixture(); f.inventory(["broken", "pi"]);
    expect(f.run("snapshot").status).toBe(0);
    writeFileSync(join(f.next, "fixture.json"), JSON.stringify({ fail: "broken@latest" }));
    const result = f.run("restore");
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("清单已保留");
    expect(readFileSync(join(f.next, "installed.txt"), "utf8")).toContain("pi@latest");
    expect(JSON.parse(readFileSync(f.manifest, "utf8")).results.map((entry: { status: string }) => entry.status)).toEqual(["failed", "succeeded"]);
  });

  it("preserves ignore-scripts and local linked sources", () => {
    const f = fixture(); f.inventory(["local-cli"], "node-pty", { ignore: true });
    const linked = join(f.base, "local project"); mkdirSync(linked);
    rmSync(join(f.old, "lib/node_modules/local-cli"), { recursive: true });
    symlinkSync(linked, join(f.old, "lib/node_modules/local-cli"));
    expect(f.run("snapshot").status).toBe(0);
    expect(f.run("restore").status).toBe(0);
    expect(f.calls(f.next)[0]).toContain("--ignore-scripts");
    expect(f.calls(f.next)[0]?.at(-1)).toBe(linked);
  });

  it("only clears verified Corepack shims when restoring pnpm", () => {
    const f = fixture(); f.inventory(["pnpm"]);
    const shim = join(f.next, "lib/node_modules/corepack/dist/pnpm.js");
    mkdirSync(dirname(shim), { recursive: true }); writeFileSync(shim, "fixture");
    symlinkSync(shim, join(f.next, "bin/pnpm"));
    writeFileSync(join(f.next, "bin/pnpx"), "unrelated executable");
    expect(f.run("snapshot").status).toBe(0);
    expect(f.run("restore").status).toBe(0);
    expect(existsSync(join(f.next, "bin/pnpm"))).toBe(false);
    expect(readFileSync(join(f.next, "bin/pnpx"), "utf8")).toBe("unrelated executable");
  });

  it("restores Corepack shims if the real package fails to install", () => {
    const f = fixture(); f.inventory(["pnpm"]);
    const shim = join(f.next, "lib/node_modules/corepack/dist/pnpm.js");
    mkdirSync(dirname(shim), { recursive: true }); writeFileSync(shim, "fixture");
    symlinkSync(shim, join(f.next, "bin/pnpm"));
    writeFileSync(join(f.next, "fixture.json"), JSON.stringify({ fail: "pnpm@latest" }));
    expect(f.run("snapshot").status).toBe(0);
    expect(f.run("restore").status).toBe(1);
    expect(realpathSync(join(f.next, "bin/pnpm"))).toBe(shim);
  });
});
