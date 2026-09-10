import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
const directory = mkdtempSync(join(tmpdir(), "dbox-bindings-"));
try {
  const generated = join(directory, "bindings.ts");
  const result = spawnSync(
    "cargo",
    [
      "run",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--features",
      "bindings",
      "--bin",
      "export-bindings",
      "--",
      generated,
    ],
    { stdio: "inherit" },
  );
  if (result.status !== 0) process.exitCode = result.status ?? 1;
  else if (
    readFileSync(generated, "utf8") !== readFileSync("src/bindings.ts", "utf8")
  ) {
    console.error(
      "Rust/TypeScript bindings are out of date. Run npm run bindings:generate.",
    );
    process.exitCode = 1;
  }
} finally {
  rmSync(directory, { recursive: true, force: true });
}
