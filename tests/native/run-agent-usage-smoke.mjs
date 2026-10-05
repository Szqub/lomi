import {
  mkdtemp,
  mkdir,
  readFile,
  writeFile,
  copyFile,
} from "node:fs/promises";
import { accessSync, constants } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { execFileSync, spawn } from "node:child_process";
import { newSession, newProject } from "../../src/model.ts";

if (process.platform !== "darwin")
  throw Error("This native agent usage runner currently supports macOS.");
const root = resolve(import.meta.dirname, "../..");
const directory = await mkdtemp(join(tmpdir(), "lomi-usage-native-"));
const identifier = `dev.lomi.usage-smoke-${Date.now()}`;
const requestedLiveMode = process.argv.includes("--live-all")
  ? "all"
  : process.argv.includes("--live-agy")
    ? "agy"
    : process.argv.includes("--live")
      ? "codex"
      : null;
const appData = join(homedir(), "Library/Application Support", identifier);
const folder = join(directory, "project");
const fixtureData = join(directory, "agy-usage.json");
const offlineAgyHome = join(directory, "agy-home");
const offlineAgySettings = join(offlineAgyHome, ".gemini/antigravity-cli");
await mkdir(folder);
await mkdir(join(directory, "codex-home"));
await mkdir(join(directory, "kimi-home"));
await mkdir(offlineAgySettings, { recursive: true });
await writeFile(join(offlineAgySettings, "settings.json"), "{}\n");
await mkdir(join(directory, "shell-home"));
await mkdir(appData, { recursive: true });
// Native executable identities exercise process inspection without launching an AI request.
for (const name of ["codex", "copilot", "claude", "cursor-agent", "kimi"]) {
  await copyFile("/bin/sleep", join(directory, name));
  execFileSync("/usr/bin/codesign", [
    "--force",
    "--sign",
    "-",
    join(directory, name),
  ]);
}

const cStringLiteral = (value) =>
  `"${value
    .replaceAll("\\", "\\\\")
    .replaceAll('"', '\\"')
    .replaceAll("\n", "\\n")
    .replaceAll("\r", "\\r")
    .replaceAll("\t", "\\t")}"`;
const envelope = (shortWindow, longWindow) =>
  JSON.stringify({
    status: "SUCCESS",
    num_turns: 0,
    conversation_id: "",
    duration_seconds: 0,
    response: "",
    usage: {
      input_tokens: 0,
      output_tokens: 0,
      thinking_tokens: 0,
      cache_read_tokens: 0,
      total_tokens: 0,
    },
    command: {
      name: "usage",
      data: {
        description: "Account quota",
        groups: [
          {
            name: "Plan",
            description: "",
            buckets: [
              {
                id: "short-window",
                name: "Short window",
                window: "5h",
                remaining_fraction: shortWindow / 100,
                reset_time: "2099-01-01T00:00:00Z",
                description: "",
              },
              {
                id: "long-window",
                name: "Long window",
                window: "weekly",
                remaining_fraction: longWindow / 100,
                reset_time: "2099-01-01T00:00:00Z",
                description: "",
              },
            ],
          },
        ],
      },
    },
  });
await writeFile(fixtureData, envelope(81, 44));

const compileAgyFixture = async (executable, realAgy = null) => {
  await mkdir(dirname(executable), { recursive: true });
  const sdkPath = execFileSync(
    "/usr/bin/xcrun",
    ["--sdk", "macosx", "--show-sdk-path"],
    { encoding: "utf8" },
  ).trim();
  const args = [
    "clang",
    "-isysroot",
    sdkPath,
    "-std=c11",
    "-O2",
    `-DAGY_FIXTURE_JSON_PATH=${cStringLiteral(fixtureData)}`,
  ];
  if (realAgy) args.push(`-DAGY_REAL_EXECUTABLE=${cStringLiteral(realAgy)}`);
  args.push(join(root, "tests/native/agent-usage-fixture.c"), "-o", executable);
  execFileSync("/usr/bin/xcrun", args);
  execFileSync("/usr/bin/codesign", ["--force", "--sign", "-", executable]);
};

const agyExecutable = join(directory, "agy-bin", "agy");
await compileAgyFixture(agyExecutable);
let liveAgyExecutable = null;
if (requestedLiveMode === "agy") {
  const candidates = [
    ...(process.env.PATH ?? "")
      .split(delimiter)
      .map((path) => join(path, "agy")),
    join(homedir(), ".local/bin/agy"),
    "/opt/homebrew/bin/agy",
  ];
  const realAgy = candidates.find((candidate) => {
    try {
      accessSync(candidate, constants.X_OK);
      return true;
    } catch {
      return false;
    }
  });
  if (!realAgy)
    throw Error(
      "--live-agy requires an installed Antigravity CLI in PATH or ~/.local/bin/agy.",
    );
  liveAgyExecutable = join(directory, "live-agy-bin", "agy");
  await compileAgyFixture(liveAgyExecutable, realAgy);
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
      LOMI_USAGE_SMOKE_AGY_EXECUTABLE: agyExecutable,
      LOMI_USAGE_SMOKE_AGY_FIXTURE_DATA: fixtureData,
      LOMI_USAGE_SMOKE_AGY_HOME: offlineAgyHome,
      ZDOTDIR: join(directory, "shell-home"),
      HISTFILE: "/dev/null",
      ...(requestedLiveMode
        ? {
            LOMI_USAGE_SMOKE_LIVE_HOME:
              process.env.CODEX_HOME || join(homedir(), ".codex"),
            LOMI_USAGE_SMOKE_LIVE_KIMI_HOME: join(homedir(), ".kimi-code"),
            ...(requestedLiveMode === "agy"
              ? {
                  LOMI_USAGE_SMOKE_LIVE_AGY_EXECUTABLE: liveAgyExecutable,
                  LOMI_USAGE_SMOKE_LIVE_AGY_HOME: homedir(),
                }
              : {}),
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
