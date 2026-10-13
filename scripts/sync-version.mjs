import { readFileSync, writeFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const changes = [];

function read(file) {
  return readFileSync(new URL(file, root), "utf8");
}

function planChange(file, before, after) {
  if (before !== after) changes.push({ file, after });
}

function updateJson(file, update) {
  const before = read(file);
  const data = JSON.parse(before);
  update(data);
  const newline = before.includes("\r\n") ? "\r\n" : "\n";
  const after = `${JSON.stringify(data, null, 2)}\n`.replaceAll("\n", newline);
  planChange(file, before, after);
}

function updateCargo(file, header, version, packageName) {
  const before = read(file);
  let matches = 0;
  let name;
  // Limit edits to the application's table, preserving dependency versions and comments.
  const after = before.split(/(?=^\[)/m).map((section) => {
    if (section.split(/\r?\n/, 1)[0].trim() !== header) return section;
    const sectionName = section.match(/^[ \t]*name[ \t]*=[ \t]*["']([^"'\r\n]+)["']/m)?.[1];
    if (packageName !== undefined && (
      sectionName !== packageName || /^[ \t]*source[ \t]*=/m.test(section)
    )) return section;
    if (!sectionName) throw new Error(`${file}: missing package name.`);
    name = sectionName;
    matches += 1;
    let versions = 0;
    const updated = section.replace(
      /^([ \t]*version[ \t]*=[ \t]*)(["'])([^"'\r\n]*)\2/gm,
      (_, prefix, quote) => {
        versions += 1;
        return `${prefix}${quote}${version}${quote}`;
      },
    );
    if (versions !== 1) throw new Error(`${file}: expected one package version.`);
    return updated;
  }).join("");
  if (matches !== 1) throw new Error(`${file}: expected one application package table.`);
  planChange(file, before, after);
  return name;
}

try {
  const args = process.argv.slice(2);
  if (args.length > 1 || (args.length === 1 && args[0] !== "--check")) {
    throw new Error("Usage: node scripts/sync-version.mjs [--check]");
  }
  const check = args[0] === "--check";
  const { version } = JSON.parse(read("package.json"));
  const numeric = "(?:0|[1-9][0-9]*)";
  const prerelease = `(?:${numeric}|[0-9]*[A-Za-z-][0-9A-Za-z-]*)`;
  const semver = new RegExp(
    `^${numeric}\\.${numeric}\\.${numeric}(?:-${prerelease}(?:\\.${prerelease})*)?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$`,
  );
  if (typeof version !== "string" || version.trim() !== version || !semver.test(version)) {
    throw new Error("package.json: version must be a valid SemVer, e.g. 0.1.3 or 0.2.0-beta.1.");
  }

  updateJson("package-lock.json", (lock) => {
    if (!lock.packages?.[""]) {
      throw new Error("package-lock.json: missing root package entry.");
    }
    lock.version = version;
    lock.packages[""].version = version;
  });
  updateJson("src-tauri/tauri.conf.json", (config) => {
    config.version = version;
  });
  const packageName = updateCargo("src-tauri/Cargo.toml", "[package]", version);
  updateCargo("src-tauri/Cargo.lock", "[[package]]", version, packageName);

  // Validate every target before writing any files; --check never writes.
  if (check && changes.length > 0) {
    console.error(`Versions differ from package.json (${version}):`);
    for (const { file } of changes) console.error(`  ${file}`);
    console.error("Run npm run version:sync to update them.");
    process.exitCode = 1;
  } else if (changes.length === 0) {
    console.log(`All application versions are already ${version}.`);
  } else {
    for (const { file, after } of changes) {
      writeFileSync(new URL(file, root), after);
      console.log(`Updated ${file} to ${version}.`);
    }
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
