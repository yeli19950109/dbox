import { spawnSync } from "node:child_process";
import { access, copyFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

if (process.platform !== "darwin") {
  throw new Error("macOS icon generation requires macOS and Xcode 26 or newer.");
}

const root = fileURLToPath(new URL("../", import.meta.url));
const env = { ...process.env };
let lookup = spawnSync("xcrun", ["--find", "actool"], { env, encoding: "utf8" });
if (lookup.status !== 0 && !env.DEVELOPER_DIR) {
  env.DEVELOPER_DIR = "/Applications/Xcode.app/Contents/Developer";
  lookup = spawnSync("xcrun", ["--find", "actool"], { env, encoding: "utf8" });
}
if (lookup.status !== 0) {
  throw new Error("Set DEVELOPER_DIR to the Developer directory in Xcode 26 or newer.");
}

const output = await mkdtemp(join(tmpdir(), "dbox-macos-icon-"));
try {
  const result = spawnSync(
    lookup.stdout.trim(),
    [
      join(root, "src-tauri/icons/AppIcon.icon"),
      "--compile", output,
      "--platform", "macosx",
      "--minimum-deployment-target", "10.13",
      "--app-icon", "AppIcon",
      "--include-all-app-icons",
      "--output-partial-info-plist", join(output, "partial.plist"),
      "--output-format", "human-readable-text",
      "--notices", "--warnings", "--errors",
    ],
    { env, stdio: "inherit" },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error("Apple app icon compilation failed.");

  await access(join(output, "Assets.car"));
  await access(join(output, "AppIcon.icns"));
  await copyFile(join(output, "Assets.car"), join(root, "src-tauri/Assets.car"));
  await copyFile(join(output, "AppIcon.icns"), join(root, "src-tauri/icons/icon.icns"));
  console.log("Updated macOS Assets.car and the legacy ICNS fallback.");
} finally {
  await rm(output, { recursive: true, force: true });
}
