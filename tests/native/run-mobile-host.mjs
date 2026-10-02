// Owned local desktop PTY companion for the native mobile UI; no mobile hooks.
import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
  rename,
  rm,
} from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";

import {
  newProject,
  newSession,
  newTab,
  newWorkspace,
} from "../../src/model.ts";
import { terminateOwnedProcessGroup } from "./remote-demo-process.mjs";
if (process.platform !== "darwin" || process.arch !== "arm64")
  throw Error("Native qualification requires macOS ARM64");
const root = resolve(import.meta.dirname, "../.."),
  fixtureDirectory = process.env.LOMI_REMOTE_E2E_DIRECTORY;
if (!fixtureDirectory || !fixtureDirectory.startsWith("/"))
  throw Error("An explicit private mobile fixture directory is required.");
let cancelled = false;
let finishCancellation;
const cancellation = new Promise((resolve) => {
  finishCancellation = resolve;
});
const cancel = () => {
  cancelled = true;
  finishCancellation();
};
process.on("SIGINT", cancel);
process.on("SIGTERM", cancel);
process.on("SIGHUP", cancel);
const fixture = JSON.parse(
  await readFile(join(fixtureDirectory, "fixture.json"), "utf8"),
);
if (
  !/^mobile_native_ui_[0-9a-f-]{36}$/.test(fixture.userId) ||
  fixture.identityOrigin !== "http://127.0.0.1:4324" ||
  fixture.apiOrigin !== "http://127.0.0.1:3002" ||
  fixture.remoteOrigin !== fixture.apiOrigin
)
  throw Error("Only the owned mobile terminal fixture is accepted.");
if (!(Date.parse(fixture.desktopExpiresAt) > Date.now()))
  throw Error(
    "The local test account has expired. Renew the isolated fixture before testing.",
  );
const directory = await mkdtemp(join(fixtureDirectory, "native-run-"));
await writeFile(join(directory, "fixture.json"), JSON.stringify(fixture), {
  mode: 0o600,
});
const identifier = `dev.lomi.remote-probe-${crypto.randomUUID()}`,
  appData = join(homedir(), "Library/Application Support", identifier),
  folder = join(directory, "project");
await mkdir(appData, { mode: 0o700 });
await mkdir(folder, { recursive: true, mode: 0o700 });
const project = newProject(folder, "local:zsh");
const workspace = project.workspaces[0];
workspace.name = "Remote test workspace";
workspace.tabs.push(newTab(folder, "local:zsh", "Background terminals", 2));
project.workspaces.push(newWorkspace(folder, "local:zsh", "Private workspace"));
await writeFile(
  join(appData, "session.json"),
  JSON.stringify({
    ...newSession(),
    sidebar: "workspaces",
    projects: [project],
    activeProjectId: project.id,
  }),
  { mode: 0o600 },
);
const port = 1448,
  config = join(directory, "native-config.json");
const base = JSON.parse(
  await readFile(join(root, "src-tauri/tauri.conf.json"), "utf8"),
);
await writeFile(
  config,
  JSON.stringify({
    identifier,
    build: {
      beforeDevCommand: `pnpm dev --port ${port} --strictPort`,
      devUrl: `http://127.0.0.1:${port}`,
    },
    app: {
      security: {
        devCsp: base.app.security.devCsp.replaceAll(
          "ws://127.0.0.1:1420",
          `ws://127.0.0.1:${port}`,
        ),
      },
    },
  }),
  { mode: 0o600 },
);
for (const name of [
  "native-state.json",
  "native-result.json",
  "native-reply.json",
  "command.json",
])
  await rm(join(directory, name), { force: true });
const child = spawn(
  "pnpm",
  [
    "tauri",
    "dev",
    "--no-watch",
    "--features",
    "remote-probe",
    "--config",
    config,
  ],
  {
    cwd: root,
    detached: true,
    env: {
      ...process.env,
      LOMI_REMOTE_PROBE_DIRECTORY: directory,
      LOMI_REMOTE_PROBE_MANUAL: "1",
      LOMI_AUTH_ORIGIN: "http://127.0.0.1:4324",
      LOMI_REMOTE_ORIGIN: "http://127.0.0.1:4322",
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
child.detached = true;
let log = "";
for (const s of [child.stdout, child.stderr])
  s.on("data", (d) => {
    if (log.length < 2 * 1024 * 1024) log += d.toString();
  });
let demoDeadline,
  seq = 0;
const wait = async (fn, timeout = 120000, allowCancelled = false) => {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (cancelled && !allowCancelled)
      throw Error("Local Remote demo cancelled.");
    const r = await fn();
    if (r) return r;
    if (child.exitCode !== null)
      throw Error("Native application exited before qualification");
    await new Promise((r) => setTimeout(r, 100));
  }
  throw Error("Qualification timeout");
};
const jsonFile = async (name) =>
  readFile(join(directory, name), "utf8")
    .then(JSON.parse)
    .catch(() => null);
const command = async (op, data = {}, cleanup = false) => {
  seq++;
  const next = join(directory, "command.next");
  await writeFile(next, JSON.stringify({ seq, op, ...data }), { mode: 0o600 });
  await rename(next, join(directory, "command.json"));
  const reply = await wait(
    async () => {
      const r = await jsonFile("native-reply.json");
      return r?.seq === seq ? r : null;
    },
    cleanup ? 3000 : 20000,
    cleanup,
  );
  assert.equal(reply.ok, true, reply.error);
  return reply.result;
};
try {
  const state = await wait(async () => {
    const result = await jsonFile("native-result.json");
    if (result && !result.passed) throw Error(result.error);
    return jsonFile("native-state.json");
  }, 600000);
  assert.equal(state.ready, true);
  // Fixture setup publishes through the real native host API. Desktop menu
  // gesture qualification belongs to the separate retained desktop UI runner.
  await command("share-workspace", { workspaceId: workspace.id, shared: true });
  await wait(
    async () =>
      (await command("inspect")).remote.workspaces?.some(
        (w) => w.id === workspace.id && w.shared && w.online,
      ),
    30000,
  );
  await writeFile(
    join(directory, "mobile-host-ready.json"),
    JSON.stringify({
      ready: true,
      workspaceId: workspace.id,
      directory,
      expiresAt: fixture.desktopExpiresAt,
    }),
    { mode: 0o600 },
  );
  console.log(
    JSON.stringify({
      kind: "mobile_native_host",
      ready: true,
      artifactDirectory: directory,
      retainedPanes: 3,
    }),
  );
  await Promise.race([
    cancellation,
    new Promise((resolve) => {
      demoDeadline = setTimeout(
        resolve,
        Math.max(1, Date.parse(fixture.desktopExpiresAt) - Date.now()),
      );
      child.once("exit", resolve);
    }),
  ]);
  if (child.exitCode === null) await command("quit", {}, true).catch(() => {});
} catch (error) {
  await writeFile(
    join(directory, "host-failure.json"),
    JSON.stringify({
      stage: "owned-host-qualification",
      error: error instanceof Error ? error.message : "Unknown failure",
    }),
    { mode: 0o600 },
  ).catch(() => {});
  console.error(
    "Owned mobile host qualification failed; inspect private local artifacts.",
  );
  process.exitCode = 1;
} finally {
  clearTimeout(demoDeadline);
  let stopped = false;
  try {
    await terminateOwnedProcessGroup(child);
    stopped = true;
  } catch {
    process.exitCode = 1;
  }
  await writeFile(join(directory, "native.log"), log, { mode: 0o600 }).catch(
    () => {},
  );
  if (stopped) {
    await rm(appData, { recursive: true, force: true }).catch(() => {});
    await rm(join(directory, "fixture.json"), { force: true }).catch(() => {});
  }
  process.removeListener("SIGINT", cancel);
  process.removeListener("SIGTERM", cancel);
  process.removeListener("SIGHUP", cancel);
}
