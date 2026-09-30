// Browser-only local fixture: reopening it leaves the existing native PTYs running.
import { readFile, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { chromium } from "@playwright/test";

const directory =
  process.env.LOMI_REMOTE_E2E_DIRECTORY || "/tmp/lomi-remote-live-e2e";
const fixture = JSON.parse(
  await readFile(join(directory, "fixture.json"), "utf8"),
);
if (
  fixture.remoteOrigin !== "http://127.0.0.1:4322" ||
  !(Date.parse(fixture.expiresAt) > Date.now())
)
  throw Error("An active isolated local browser fixture is required.");
let finish;
const stopped = new Promise((resolve) => {
  finish = resolve;
});
const cancel = () => finish();
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"])
  process.on(signal, cancel);
let browser;
const receipt = join(directory, `browser-window-${process.pid}.json`);
try {
  browser = await chromium.launch({
    headless: false,
    handleSIGINT: false,
    handleSIGTERM: false,
    handleSIGHUP: false,
  });
  const context = await browser.newContext();
  await context.addCookies([
    {
      name: fixture.browserCookieName,
      value: fixture.browserToken,
      url: fixture.remoteOrigin,
      httpOnly: true,
      sameSite: "Lax",
    },
  ]);
  const page = await context.newPage();
  await page.goto(fixture.remoteOrigin);
  await page.getByRole("button", { name: "Workspaces", exact: true }).waitFor();
  browser.once("disconnected", cancel);
  context.once("close", cancel);
  page.once("close", cancel);
  await writeFile(
    receipt,
    JSON.stringify({
      ready: true,
      pid: process.pid,
      webSessionId: fixture.webSessionId,
      startedAt: new Date().toISOString(),
    }),
    { mode: 0o600 },
  );
  console.log(
    JSON.stringify({
      kind: "remote_browser_demo",
      ready: true,
      url: fixture.remoteOrigin,
    }),
  );
  await stopped;
} finally {
  if (browser) await browser.close().catch(() => {});
  await rm(receipt, { force: true }).catch(() => {});
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"])
    process.removeListener(signal, cancel);
}
