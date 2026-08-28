import { spawn } from "node:child_process";

const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const cargoCommand = process.platform === "win32" ? "cargo.exe" : "cargo";

const processes = [
  {
    name: "Vite",
    child: spawn(npmCommand, ["run", "dev"], {
      cwd: process.cwd(),
      env: process.env,
      stdio: "inherit",
    }),
  },
  {
    name: "Dev HTTP",
    child: spawn(
      cargoCommand,
      [
        "run",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--no-default-features",
        "--features",
        "dev-http",
        "--bin",
        "dev-http",
      ],
      {
        cwd: process.cwd(),
        env: process.env,
        stdio: "inherit",
      },
    ),
  },
];

let stopping = false;
let exitCode = 0;

function stop(signal, code) {
  if (stopping) return;
  stopping = true;
  exitCode = code;
  for (const { child } of processes) {
    if (child.exitCode === null && child.signalCode === null) child.kill(signal);
  }
}

process.on("SIGINT", () => stop("SIGINT", 130));
process.on("SIGTERM", () => stop("SIGTERM", 143));

const completions = processes.map(
  ({ name, child }) =>
    new Promise((resolve) => {
      child.once("error", (error) => {
        console.error(`${name} failed to start: ${error.message}`);
        stop("SIGTERM", 1);
      });
      child.once("close", (code, signal) => {
        if (!stopping) {
          console.error(
            `${name} stopped${signal ? ` from ${signal}` : ` with exit code ${code ?? 1}`}`,
          );
          stop("SIGTERM", code && code !== 0 ? code : 1);
        }
        resolve();
      });
    }),
);

await Promise.all(completions);
process.exitCode = exitCode;
