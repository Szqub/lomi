import { expect, test } from "@playwright/test";
import { mockDesktop } from "./desktop";

const installedAgents = [
  { cli: "cursor", name: "Cursor CLI", command: "cursor-agent" },
  { cli: "claude", name: "Claude Code", command: "claude" },
];

async function openAgents(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  return page.getByRole("dialog", { name: "Agents" });
}

async function setInstalledAgents(
  page: import("@playwright/test").Page,
  agents = installedAgents,
) {
  await page.evaluate((value) => {
    (window as any).__nativeTest.installedAgentClis = value;
  }, agents);
}

test("Agents lists detected CLIs and starts four terminals in the project", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page, false);
  await page.goto("/");
  await setInstalledAgents(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.installedAgentCliDelay = 120;
  });
  const dialog = await openAgents(page);

  await expect(dialog.getByRole("status")).toHaveText(
    "Looking for installed agents…",
  );
  await expect(
    dialog.getByRole("button", { name: "Refresh installed CLI" }),
  ).toBeDisabled();
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await expect(dialog).toContainText("Cursor CLI");
  await expect(dialog).toContainText("cursor-agent");
  await expect(dialog).toContainText("Claude Code");
  await expect(dialog).toContainText("claude");
  await expect(dialog).not.toContainText("Aider");
  await expect(dialog.getByLabel("Number of terminals")).toHaveValue("4");
  await expect(dialog.locator(".agents-directory code")).toHaveText("/project");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBeGreaterThanOrEqual(1);
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.installedAgentCliCalls,
    ),
  ).toEqual(
    expect.arrayContaining([{ profileId: "local:bash", cwd: "/project" }]),
  );
  await dialog.screenshot({
    path: testInfo.outputPath("agents-dialog-light.png"),
  });

  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("tab")).toHaveCount(5);
  await expect(page.getByRole("tab", { name: "Cursor CLI 1" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  for (const suffix of [2, 3, 4])
    await expect(
      page.getByRole("tab", { name: `Cursor CLI ${suffix}` }),
    ).toHaveAttribute("aria-selected", "false");

  const agentStarts = () =>
    page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) => call.command === "start_terminal")
        .map((call: any) => call.args.request)
        .filter((request: any) => request.cliLaunch),
    );
  await expect.poll(async () => (await agentStarts()).length).toBe(4);
  const requests = await agentStarts();
  expect(requests.map((request: any) => request.cliLaunch)).toEqual([
    "cursor",
    "cursor",
    "cursor",
    "cursor",
  ]);
  expect(requests.map((request: any) => request.cwd)).toEqual([
    "/project",
    "/project",
    "/project",
    "/project",
  ]);
  expect(requests.map((request: any) => request.profileId)).toEqual([
    "local:bash",
    "local:bash",
    "local:bash",
    "local:bash",
  ]);
  expect(new Set(requests.map((request: any) => request.id)).size).toBe(4);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "write_terminal",
      ),
    ),
  ).toEqual([]);

  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: "Cursor CLI 1" }).click();
  expect((await agentStarts()).length).toBe(4);

  await expect
    .poll(() =>
      page.evaluate(() => {
        const saved = JSON.parse(
          localStorage.getItem("test-session") ?? "null",
        );
        return saved?.projects?.[0]?.workspaces?.[0]?.tabs?.length ?? 0;
      }),
    )
    .toBe(5);
  expect(
    await page.evaluate(() =>
      JSON.stringify(
        JSON.parse(localStorage.getItem("test-session") ?? "null"),
      ).includes('"cliLaunch"'),
    ),
  ).toBe(false);

  await page.reload();
  await expect(page.getByRole("tab")).toHaveCount(5);
  for (const suffix of [1, 2, 3, 4]) {
    await page.getByRole("tab", { name: `Cursor CLI ${suffix}` }).click();
    await expect(page.locator(".terminal-host")).toBeVisible();
  }
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls
        .filter((call: any) => call.command === "start_terminal")
        .map((call: any) => call.args.request.cliLaunch)
        .filter(Boolean),
    ),
  ).toEqual([]);
});

test("Agents handles empty and failed discovery and can retry", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("status")).toHaveText(
    "No supported agent CLI found. Install a CLI, then refresh.",
  );
  await expect(
    dialog.getByRole("button", { name: "Launch agents" }),
  ).toBeDisabled();

  await page.evaluate(() => {
    (window as any).__nativeTest.installedAgentCliError =
      "Agent detection is unavailable";
  });
  await dialog.getByRole("button", { name: "Refresh installed CLI" }).click();
  await expect(dialog.getByRole("alert")).toHaveText(
    "Agent detection is unavailable",
  );

  await page.evaluate((agents) => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliError = "";
    mock.installedAgentClis = agents;
  }, installedAgents);
  await dialog.getByRole("button", { name: "Refresh installed CLI" }).click();
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  const discoveryCalls = await page.evaluate(
    () => (window as any).__nativeTest.installedAgentCliCalls,
  );
  expect(discoveryCalls.length).toBeGreaterThanOrEqual(3);
  expect(discoveryCalls).toEqual(
    Array.from(discoveryCalls, () => ({
      profileId: "local:bash",
      cwd: "/project",
    })),
  );
});

test("Agents cancel and Escape restore focus to the New tab button", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await setInstalledAgents(page);
  const trigger = page.getByRole("button", { name: /^New tab/ });

  let dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(trigger).toBeFocused();

  dialog = await openAgents(page);
  await expect(dialog).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test("Agents validates the terminal count and supports another installed CLI", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await mockDesktop(page, false);
  await page.goto("/");
  await setInstalledAgents(page);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("radio", { name: /Claude Code/ }).check();
  await expect(dialog.getByLabel("Number of terminals")).toHaveValue("4");
  await page.setViewportSize({ width: 560, height: 640 });
  await dialog.screenshot({
    path: testInfo.outputPath("agents-dialog-dark-small.png"),
  });

  const count = dialog.getByLabel("Number of terminals");
  await count.fill("1.5");
  expect(
    await count.evaluate((input: HTMLInputElement) => input.checkValidity()),
  ).toBe(false);
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" && call.args.request.cliLaunch,
      ),
    ),
  ).toEqual([]);

  await count.fill("0");
  expect(
    await count.evaluate((input: HTMLInputElement) => input.checkValidity()),
  ).toBe(false);
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toBeVisible();

  await count.fill("2");
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("tab")).toHaveCount(3);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) =>
              call.command === "start_terminal" &&
              call.args.request.cliLaunch === "claude",
          ).length,
      ),
    )
    .toBe(2);
});

test("a native agent startup error is visible and does not trigger a duplicate start", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await setInstalledAgents(page, [installedAgents[0]]);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(1);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentStartupFailures = 1;
  });
  await dialog.getByRole("button", { name: "Launch agents" }).click();

  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("alert")).toContainText(
    "1 of 4 agents could not start. Agent CLI failed to start",
  );
  const cliRequests = () =>
    page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" && call.args.request.cliLaunch,
      ),
    );
  await expect.poll(async () => (await cliRequests()).length).toBe(4);
  await page.getByRole("tab", { name: "Cursor CLI 1" }).click();
  for (const suffix of [1, 2, 3, 4])
    await page.getByRole("tab", { name: `Cursor CLI ${suffix}` }).click();
  await expect(page.getByRole("alert")).toContainText(
    "1 of 4 agents could not start",
  );
  expect((await cliRequests()).length).toBe(4);
});
