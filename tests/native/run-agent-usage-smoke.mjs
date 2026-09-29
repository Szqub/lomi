import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  copyFile,
} from "node:fs/promises";
import { tmpdir, homedir } from "node:os";
import { join, resolve } from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This native agent usage runner currently supports macOS.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-usage-native-"));
const identifier = `dev.lomi.usage-smoke-${Date.now()}`;
const requestedLiveMode = process.argv.includes("--live-all")
  ? "all"
  : process.argv.includes("--live")
    ? "codex"
    : null;
const appData = join(homedir(), "Library/Application Support", identifier);
const folder = join(directory, "project");
await mkdir(folder);
await mkdir(join(directory, "codex-home"));
await mkdir(join(directory, "kimi-home"));
await mkdir(join(directory, "shell-home"));
await mkdir(appData, { recursive: true });
// Native executable identities exercise process inspection without launching an AI request.
for (const name of ["codex", "aider", "claude", "cursor-agent", "kimi"]) {
  await copyFile("/bin/sleep", join(directory, name));
  execFileSync("/usr/bin/codesign", [
    "--force",
    "--sign",
    "-",
    join(directory, name),
  ]);
}
const project = newProject(folder, "local:zsh");
const tab = project.workspaces[0].tabs[0];
tab.layout.id = "usage-terminal";
tab.activePaneId = tab.layout.id;
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  }),
);
const config = join(directory, "config.json");
const port = Number(process.env.LOMI_USAGE_SMOKE_PORT ?? 1439);
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm dev --port ${port}`,
      devUrl: `http://127.0.0.1:${port}`,
    },
  }),
);
console.log(
  `Native usage artifacts: ${directory}\nIsolated application data: ${appData}`,
);
const child = spawn(
  "pnpm",
  [
    "tauri",
    "dev",
    "--no-watch",
    "--features",
    "native-smoke",
    "--config",
    config,
  ],
  {
    cwd: root,
    env: {
      ...process.env,
      LOMI_USAGE_SMOKE_DIRECTORY: directory,
      ZDOTDIR: join(directory, "shell-home"),
      HISTFILE: "/dev/null",
      ...(requestedLiveMode
        ? {
            LOMI_USAGE_SMOKE_LIVE_HOME:
              process.env.CODEX_HOME || join(homedir(), ".codex"),
            LOMI_USAGE_SMOKE_LIVE_KIMI_HOME: join(homedir(), ".kimi-code"),
            LOMI_USAGE_SMOKE_LIVE_MODE: requestedLiveMode,
          }
        : {}),
    },
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let log = "";
const childClosed = new Promise((resolve) => child.once("close", resolve));
for (const stream of [child.stdout, child.stderr])
  stream.on("data", (chunk) => {
    log += chunk;
    process.stdout.write(chunk);
  });
let exitCode = 0;
try {
  const deadline = Date.now() + 300000;
  let result;
  while (!result && Date.now() < deadline) {
    try {
      result = JSON.parse(
        await readFile(join(directory, "result.json"), "utf8"),
      );
    } catch (error) {
      if (error.code !== "ENOENT" && !(error instanceof SyntaxError))
        throw error;
    }
    if (!result) {
      if (child.exitCode !== null)
        throw Error(`Tauri exited before reporting: ${child.exitCode}`);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  if (!result) throw Error("Native usage smoke timed out.");
  console.log(JSON.stringify(result, null, 2));
  if (result.stage !== "passed" && result.data?.offlineSmoke === "passed") {
    console.error(
      "Offline native smoke passed; the live account usage probe failed.",
    );
    exitCode = 2;
  } else if (result.stage !== "passed") exitCode = 1;
  else if (
    requestedLiveMode &&
    result.data?.liveProbe?.mode !== requestedLiveMode
  ) {
    console.error(
      "Offline native smoke passed; the requested live usage results were not reported.",
    );
    exitCode = 2;
  } else if (result.data?.liveProbe?.status === "incomplete") {
    console.error(
      "Offline native smoke passed; live account usage verification was incomplete.",
    );
    exitCode = 2;
  }
} finally {
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
  let closeTimer;
  await Promise.race([
    childClosed,
    new Promise((resolve) => {
      closeTimer = setTimeout(resolve, 3000);
      closeTimer.unref();
    }),
  ]);
  clearTimeout(closeTimer);
  await writeFile(join(directory, "native.log"), log);
}
process.exitCode = exitCode;
