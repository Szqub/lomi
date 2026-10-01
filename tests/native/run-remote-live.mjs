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
import { chromium } from "@playwright/test";
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
  fixtureDirectory =
    process.env.LOMI_REMOTE_E2E_DIRECTORY || "/tmp/lomi-remote-live-e2e";
const manual = process.argv.includes("--manual");
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
const port = manual ? 1448 : 1449,
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
      LOMI_REMOTE_PROBE_MANUAL: manual ? "1" : "0",
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
let browser,
  context,
  page,
  demoDeadline,
  seq = 0;
const checks = [];
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
  console.log(
    JSON.stringify({
      kind: "remote_native_e2e",
      status: "building",
      identifier,
      artifactDirectory: directory,
    }),
  );
  const state = await wait(async () => {
    const result = await jsonFile("native-result.json");
    if (result && !result.passed) throw Error(result.error);
    return jsonFile("native-state.json");
  }, 600000);
  assert.equal(state.ready, true);
  checks.push("real retained desktop PTY and native host enabled");
  browser = await chromium.launch({
    headless: !manual,
    handleSIGINT: false,
    handleSIGTERM: false,
    handleSIGHUP: false,
  });
  context = await browser.newContext();
  page = await context.newPage();
  await page.goto("http://127.0.0.1:4322");
  await page.getByRole("link", { name: "Continue with Lomi" }).waitFor();
  assert.equal(
    await page.getByRole("button", { name: "Workspaces", exact: true }).count(),
    0,
  );
  checks.push("unauthenticated browser exposes login only");
  await context.addCookies([
    {
      name: fixture.browserCookieName,
      value: fixture.browserToken,
      url: "http://127.0.0.1:4322",
      httpOnly: true,
      sameSite: "Lax",
    },
  ]);
  await page.reload();
  await page.getByRole("button", { name: "Workspaces", exact: true }).waitFor();
  if (manual) {
    await writeFile(
      join(directory, "demo-ready.json"),
      JSON.stringify({
        ready: true,
        url: fixture.remoteOrigin,
        workspaceId: workspace.id,
      }),
      { mode: 0o600 },
    );
    console.log(
      JSON.stringify({
        kind: "remote_manual_demo",
        ready: true,
        url: "http://127.0.0.1:4322",
        expiresAt: fixture.desktopExpiresAt,
        instructions: [
          "The visible browser and isolated Lomi desktop use the same local test account.",
          "Desktop: right-click Remote test workspace → Share remotely.",
          "Browser: Workspaces → Open workspace; all three terminals are available automatically.",
          "New terminal panes join the shared workspace automatically; Stop sharing removes browser access.",
          "Close the desktop window to test background sessions; use the Dock to reopen it.",
          "Press Ctrl+C in this terminal to stop the local demo.",
        ],
      }),
    );
    await Promise.race([
      cancellation,
      new Promise((resolve) => {
        const finish = () => {
          clearTimeout(timer);
          resolve();
        };
        const timer = (demoDeadline = setTimeout(
          finish,
          Math.max(1, Date.parse(fixture.desktopExpiresAt) - Date.now()),
        ));
        child.once("exit", finish);
      }),
    ]);
    if (child.exitCode === null)
      await command("quit", {}, true).catch(() => {});
  } else {
    await command("ui-workspace-action", {
      workspaceId: workspace.id,
      action: "share",
    });
    await wait(async () => {
      const r = await command("inspect");
      return r.remote.workspaces?.find(
        (w) => w.id === workspace.id && w.shared && w.online,
      );
    }, 30000);
    const response = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      {
        headers: { "X-Lomi-Request": "1" },
      },
    );
    const publicWorkspaces = (await response.json()).workspaces;
    const published = publicWorkspaces.find(
      (w) => w.id === workspace.id && w.hostId === state.remote.hostId,
    );
    assert.ok(
      published,
      "Native shared workspace is discoverable by the same account",
    );
    assert.equal(
      published.sessionIds.length,
      3,
      "Inactive split panes are shared together",
    );
    assert.equal(
      publicWorkspaces.some(
        (w) =>
          w.id === project.workspaces[1].id && w.hostId === state.remote.hostId,
      ),
      false,
    );
    const publicWire = JSON.stringify(publicWorkspaces);
    assert.equal(publicWire.includes(workspace.name), false);
    assert.equal(publicWire.includes(folder), false);
    checks.push(
      "one native workspace menu action starts and shares all three terminal panes",
    );
    checks.push(
      "unshared workspace excluded and cloud inventory contains no private names or paths",
    );
    const card = page
      .locator(".resource-list li")
      .filter({ hasText: workspace.name });
    await card.getByRole("button", { name: "Open", exact: true }).click();
    await page.getByRole("group", { name: "Workspace terminals" }).waitFor();
    assert.equal(await page.locator(".terminal-tabs button").count(), 3);
    assert.equal(
      await page.getByText("Full pairing fingerprint", { exact: true }).count(),
      0,
    );
    await page
      .getByRole("button", { name: "Renew control", exact: true })
      .waitFor();
    await wait(
      async () =>
        await page
          .locator(".xterm-screen")
          .innerText()
          .then((v) => v.includes("LOMI_NATIVE_REMOTE_READY"))
          .catch(() => false),
      30000,
    );
    checks.push(
      "automatic account enrollment and encrypted native snapshot without token exchange",
    );
    const input = page.locator(".xterm-helper-textarea");
    await input.waitFor({ state: "attached" });
    await input.focus();
    await page.keyboard.type("printf 'LOMI_BROWSER_INPUT_OK\\n'");
    await page.keyboard.press("Enter");
    await wait(
      async () =>
        await page
          .locator(".xterm-screen")
          .innerText()
          .then((v) => v.includes("LOMI_BROWSER_INPUT_OK"))
          .catch(() => false),
      30000,
    );
    checks.push("leased browser input writes real native PTY");
    await command("ui-terminal-action", { action: "new-tab" });
    await wait(
      async () => (await page.locator(".terminal-tabs button").count()) === 4,
      30000,
    );
    await page
      .locator(".terminal-status")
      .filter({ hasText: "Observing. Input is disabled." })
      .waitFor();
    await page
      .getByRole("button", { name: "Take control", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Renew control", exact: true })
      .waitFor();
    await command("ui-terminal-action", { action: "split" });
    await wait(
      async () => (await page.locator(".terminal-tabs button").count()) === 5,
      30000,
    );
    await page
      .locator(".terminal-status")
      .filter({ hasText: "Observing. Input is disabled." })
      .waitFor();
    await page
      .getByRole("button", { name: "Take control", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Renew control", exact: true })
      .waitFor();
    checks.push(
      "new terminal tab and split update signed scope, reconnect observing, and accept explicit control",
    );
    const selectedUrl = page.url();
    await page.reload();
    await page
      .getByRole("button", { name: "Renew control", exact: true })
      .waitFor();
    assert.equal(page.url(), selectedUrl);
    await wait(
      async () =>
        await page
          .locator(".xterm-screen")
          .innerText()
          .then((v) => v.includes("LOMI_BROWSER_INPUT_OK"))
          .catch(() => false),
      30000,
    );
    checks.push(
      "browser refresh automatically re-enrolls and restores selected terminal snapshot",
    );
    await command("close-window");
    await wait(async () => !(await command("inspect")).windowVisible, 10000);
    await input.focus();
    await page.keyboard.type("printf 'LOMI_HIDDEN_NATIVE_OK\\n'");
    await page.keyboard.press("Enter");
    await wait(
      async () =>
        await page
          .locator(".xterm-screen")
          .innerText()
          .then((v) => v.includes("LOMI_HIDDEN_NATIVE_OK"))
          .catch(() => false),
      30000,
    );
    checks.push(
      "desktop window close keeps renderer and native sessions alive",
    );
    await command("local-input", { data: "printf 'LOMI_LOCAL_PRIORITY\\n'\n" });
    await wait(
      async () =>
        await page.getByRole("button", { name: "Take control" }).isVisible(),
      15000,
    );
    checks.push("local human input invalidates remote lease");
    await page.getByRole("button", { name: "Activity", exact: true }).click();
    await page.getByRole("button", { name: "Workspaces", exact: true }).click();
    assert.equal(
      await page.getByRole("region", { name: "Shared terminal" }).count(),
      1,
    );
    checks.push("navigation retains connected terminal");
    await command("reopen-window");
    await page
      .getByRole("button", { name: "Take control", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Renew control", exact: true })
      .waitFor();
    await command("ui-workspace-action", {
      workspaceId: workspace.id,
      action: "stop",
    });
    await wait(async () => {
      const r = await command("inspect");
      return r.remote.workspaces?.find(
        (w) => w.id === workspace.id && !w.shared,
      );
    }, 10000);
    await wait(
      async () =>
        !(await page
          .getByRole("button", { name: "Renew control", exact: true })
          .count()),
      15000,
    );
    const stoppedInventory = await page.request.get(
      "http://127.0.0.1:4322/v1/remote/workspaces",
      {
        headers: { "X-Lomi-Request": "1" },
      },
    );
    assert.equal(
      (await stoppedInventory.json()).workspaces.some(
        (w) => w.id === workspace.id && w.hostId === state.remote.hostId,
      ),
      false,
    );
    await command("local-input", {
      data: "printf 'LOMI_LOCAL_AFTER_UNSHARE\\n'\n",
    });
    checks.push("Stop sharing fences browser access while local PTYs continue");
    assert.equal(
      (await command("inspect")).remote.sessions.filter((s) => s.available)
        .length,
      5,
    );
    await command("ui-workspace-action", {
      workspaceId: workspace.id,
      action: "share",
    });
    await card.getByRole("button", { name: "Open", exact: true }).click();
    await page
      .getByRole("button", { name: "Renew control", exact: true })
      .waitFor();
    assert.equal(await page.locator(".terminal-tabs button").count(), 5);
    checks.push(
      "re-sharing enrolls a fresh scope automatically without refreshing the browser",
    );
    const current = await command("inspect");
    const grant = current.remote.grants.find((g) => !g.revoked);
    assert.ok(grant);
    await command("revoke", { grantId: grant.id });
    await wait(
      async () =>
        (await page.getByText("Access revoked", { exact: true }).count()) > 0,
      15000,
    );
    await new Promise((resolve) => setTimeout(resolve, 3000));
    assert.equal(
      await page
        .getByRole("button", { name: "Renew control", exact: true })
        .count(),
      0,
    );
    checks.push(
      "explicit browser revoke stays revoked instead of automatically re-enrolling",
    );

    await command("quit");
    await wait(async () => await jsonFile("native-result.json"), 10000);
    await new Promise((resolve, reject) => {
      if (child.exitCode !== null) return resolve();
      const timer = setTimeout(
        () => reject(Error("Explicit quit did not exit the desktop")),
        15000,
      );
      child.once("exit", () => {
        clearTimeout(timer);
        resolve();
      });
    });
    checks.push("explicit quit stops native sessions");
    await writeFile(
      join(directory, "receipt.json"),
      JSON.stringify(
        {
          kind: "remote_native_browser_e2e",
          platform: "macOS ARM64",
          engine: "Chromium",
          passed: true,
          checks,
        },
        null,
        2,
      ) + "\n",
      { mode: 0o600 },
    );
    console.log(
      JSON.stringify({
        kind: "remote_native_browser_e2e",
        passed: true,
        checks,
      }),
    );
  }
} catch (error) {
  await page
    ?.screenshot({ path: join(directory, "failure.png"), fullPage: true })
    .catch(() => {});
  await writeFile(
    join(directory, "receipt.json"),
    JSON.stringify(
      {
        kind: "remote_native_browser_e2e",
        passed: false,
        error: error.message,
        checks,
      },
      null,
      2,
    ) + "\n",
    { mode: 0o600 },
  );
  if (!cancelled) throw error;
  console.log(JSON.stringify({ kind: "remote_manual_demo", stopped: true }));
} finally {
  clearTimeout(demoDeadline);
  let stopped = false;
  try {
    await terminateOwnedProcessGroup(child);
    stopped = true;
  } catch {
    process.exitCode = 1;
    console.error(
      "The owned demo process group could not be stopped; its app data was retained.",
    );
  }
  if (browser) {
    let closeDeadline;
    await Promise.race([
      browser.close().catch(() => {}),
      new Promise((resolve) => {
        closeDeadline = setTimeout(resolve, 5000);
      }),
    ]);
    clearTimeout(closeDeadline);
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
