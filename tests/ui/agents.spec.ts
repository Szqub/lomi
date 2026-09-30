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

async function setupAgents(
  page: import("@playwright/test").Page,
  agents = installedAgents,
  blocked = false,
) {
  await mockDesktop(page, false, undefined, undefined, undefined, undefined, {
    installed: agents,
    blocked,
  });
  await page.goto("/");
}

async function releaseScans(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliBlocked = false;
    mock.installedAgentCliReleases
      .splice(0)
      .forEach((release: () => void) => release());
  });
}

test("Agents starts four terminals in a two-by-two grid within one new tab", async ({
  page,
}, testInfo) => {
  await page.emulateMedia({ colorScheme: "light" });
  await setupAgents(page, installedAgents, true);
  const dialog = await openAgents(page);

  await expect(dialog.getByRole("status")).toHaveText(
    "Looking for installed agents…",
  );
  await expect(
    dialog.getByRole("button", { name: "Refresh installed CLI" }),
  ).toBeDisabled();
  await releaseScans(page);
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
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(
    page.getByRole("tab", { name: "Cursor CLI", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".terminal-pane")).toHaveCount(4);
  const panels = await page.locator(".terminal-pane").evaluateAll((panels) =>
    panels.map((panel) => {
      const { x, y, width, height } = panel.getBoundingClientRect();
      return { x, y, width, height };
    }),
  );
  expect(panels[0].y).toBeCloseTo(panels[1].y, 0);
  expect(panels[2].y).toBeCloseTo(panels[3].y, 0);
  expect(panels[0].x).toBeCloseTo(panels[2].x, 0);
  expect(panels[1].x).toBeCloseTo(panels[3].x, 0);
  expect(panels[1].x).toBeGreaterThan(panels[0].x);
  expect(panels[2].y).toBeGreaterThan(panels[0].y);
  for (const panel of panels) {
    expect(panel.width).toBeGreaterThanOrEqual(240);
    expect(panel.height).toBeGreaterThanOrEqual(120);
    expect(panel.width).toBeCloseTo(panels[0].width, 0);
    expect(panel.height).toBeCloseTo(panels[0].height, 0);
  }

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
  await page.screenshot({ path: testInfo.outputPath("agents-grid-light.png") });

  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: "Cursor CLI", exact: true }).click();
  await expect(page.locator(".terminal-pane")).toHaveCount(4);
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
    .toBe(2);
  expect(
    await page.evaluate(() =>
      JSON.stringify(
        JSON.parse(localStorage.getItem("test-session") ?? "null"),
      ).includes('"cliLaunch"'),
    ),
  ).toBe(false);

  await page.reload();
  await expect(page.getByRole("tab")).toHaveCount(2);
  await page.getByRole("tab", { name: "Cursor CLI", exact: true }).click();
  await expect(page.locator(".terminal-host")).toHaveCount(4);
  for (const host of await page.locator(".terminal-host").all())
    await expect(host).toBeVisible();
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
  await setupAgents(page);
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
  await setupAgents(page);
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
  await page.setViewportSize({ width: 1440, height: 900 });
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect(
    page.getByRole("tab", { name: "Claude Code", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".terminal-pane")).toHaveCount(2);
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
  await setupAgents(page, [installedAgents[0]]);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(1);
  await page.evaluate(() => {
    (window as any).__nativeTest.agentStartupFailures = 1;
  });
  await dialog.getByRole("button", { name: "Launch agents" }).click();

  await expect(dialog).toHaveCount(0);
  const startupError = page.getByRole("alert").filter({
    hasText: "1 of 4 agents could not start",
  });
  await expect(startupError).toContainText(
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
  await page.getByRole("tab", { name: "Terminal", exact: true }).click();
  await page.getByRole("tab", { name: "Cursor CLI", exact: true }).click();
  for (const panel of await page.locator(".terminal-pane").all())
    await panel.click({ position: { x: 10, y: 10 } });
  await expect(startupError).toContainText("1 of 4 agents could not start");
  expect((await cliRequests()).length).toBe(4);
});

for (const count of [1, 3, 5]) {
  test(`Agents starts exactly ${count} CLI panels in one new tab`, async ({
    page,
  }) => {
    await setupAgents(page, [installedAgents[0]]);
    const dialog = await openAgents(page);
    await expect(dialog.getByRole("radio")).toHaveCount(1);
    await dialog.getByLabel("Number of terminals").fill(String(count));
    await dialog.getByRole("button", { name: "Launch agents" }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole("tab")).toHaveCount(2);
    await expect(page.locator(".terminal-pane")).toHaveCount(count);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as any).__nativeTest.calls.filter(
              (call: any) =>
                call.command === "start_terminal" &&
                call.args.request.cliLaunch === "cursor",
            ).length,
        ),
      )
      .toBe(count);
    for (const panel of await page.locator(".terminal-pane").all())
      await expect(panel).toBeVisible();
  });
}

test("startup prewarms discovery and reopening uses the completed scan immediately", async ({
  page,
}) => {
  await setupAgents(page, installedAgents, true);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBe(1);
  await expect(page.getByRole("dialog", { name: "Agents" })).toHaveCount(0);
  await releaseScans(page);
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const { cachedInstalledAgentClis } = await import(
          performance
            .getEntriesByType("resource")
            .map((entry) => entry.name)
            .filter(
              (url) => new URL(url).pathname === "/src/installed-agent-clis.ts",
            )
            .at(-1)!
        );
        const info = await (window as any).__TAURI_INTERNALS__.invoke(
          "app_info",
        );
        return cachedInstalledAgentClis(info.profiles[0], "/project")?.length;
      }),
    )
    .toBe(2);
  for (let index = 0; index < 2; index++) {
    const dialog = await openAgents(page);
    // Inspect the first rendered dialog without waiting for discovery to finish.
    expect(await dialog.getByRole("radio").count()).toBe(2);
    await expect(
      dialog.getByRole("button", { name: "Launch agents" }),
    ).toBeEnabled();
    await expect(
      dialog.getByRole("radio", { name: /Cursor CLI/ }),
    ).toBeChecked();
    await dialog.getByRole("button", { name: "Cancel" }).click();
  }
  expect(
    await page.evaluate(
      () => (window as any).__nativeTest.installedAgentCliCalls.length,
    ),
  ).toBe(1);
});

test("refresh retains usable choices and selection through pending discovery and errors", async ({
  page,
}) => {
  await setupAgents(page);
  const dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("radio", { name: /Claude Code/ }).check();
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliBlocked = true;
    mock.installedAgentCliError = "Refresh failed";
  });
  await dialog.getByRole("button", { name: "Refresh installed CLI" }).click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Refreshing installed agents…",
  );
  expect(await dialog.getByRole("radio").count()).toBe(2);
  await expect(
    dialog.getByRole("radio", { name: /Claude Code/ }),
  ).toBeChecked();
  await expect(
    dialog.getByRole("button", { name: "Launch agents" }),
  ).toBeEnabled();
  await expect(
    dialog.getByRole("button", { name: "Refresh installed CLI" }),
  ).toBeDisabled();
  await releaseScans(page);
  await expect(dialog.getByRole("alert")).toHaveText("Refresh failed");
  await expect(
    dialog.getByRole("radio", { name: /Claude Code/ }),
  ).toBeChecked();
  await expect(
    dialog.getByRole("button", { name: "Launch agents" }),
  ).toBeEnabled();
  await dialog.getByLabel("Number of terminals").fill("1");
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "Claude Code", exact: true }),
  ).toBeVisible();
});

test("cache isolates directories and complete shell profiles and refresh waits for pending scans", async ({
  page,
}) => {
  await setupAgents(page, [], true);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBe(1);
  const observed = await page.evaluate(async () => {
    const service = await import(
      performance
        .getEntriesByType("resource")
        .map((entry) => entry.name)
        .filter(
          (url) => new URL(url).pathname === "/src/installed-agent-clis.ts",
        )
        .at(-1)!
    );
    const mock = (window as any).__nativeTest;
    const info = await (window as any).__TAURI_INTERNALS__.invoke("app_info");
    const profile = info.profiles[0];
    const first = service.loadInstalledAgentClis(profile, "/project");
    const second = service.loadInstalledAgentClis(profile, "/project");
    const refreshed = service.loadInstalledAgentClis(profile, "/project", true);
    const forcedAgain = service.loadInstalledAgentClis(
      profile,
      "/project",
      true,
    );
    const before = mock.installedAgentCliCalls.length;
    mock.installedAgentCliBlocked = false;
    mock.installedAgentCliReleases
      .splice(0)
      .forEach((release: () => void) => release());
    await refreshed;
    const after = mock.installedAgentCliCalls.length;
    const emptyCached = service.cachedInstalledAgentClis(profile, "/project");
    await service.loadInstalledAgentClis(profile, "/project");
    const emptyReused = mock.installedAgentCliCalls.length === after;
    const otherDirectory = service.cachedInstalledAgentClis(profile, "/other");
    const modifiedProfile = service.cachedInstalledAgentClis(
      { ...profile, program: "/other/bash" },
      "/project",
    );
    await service.loadInstalledAgentClis(profile, "/other");
    await service.loadInstalledAgentClis(
      { ...profile, program: "/other/bash" },
      "/project",
    );
    return {
      deduplicated: first === second,
      coalesced: refreshed === forcedAgain,
      before,
      after,
      emptyCached,
      emptyReused,
      otherDirectory,
      modifiedProfile,
      calls: mock.installedAgentCliCalls,
    };
  });
  expect(observed).toMatchObject({
    deduplicated: true,
    coalesced: true,
    before: 1,
    after: 2,
    emptyCached: [],
    emptyReused: true,
    otherDirectory: undefined,
    modifiedProfile: undefined,
  });
  expect(observed.calls).toEqual([
    { profileId: "local:bash", cwd: "/project" },
    { profileId: "local:bash", cwd: "/project" },
    { profileId: "local:bash", cwd: "/other" },
    { profileId: "local:bash", cwd: "/project" },
  ]);
});

test("stale cached choices remain launchable during background revalidation", async ({
  page,
}) => {
  await setupAgents(page);
  let dialog = await openAgents(page);
  await expect(dialog.getByRole("radio")).toHaveCount(2);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await page.evaluate(() => {
    const mock = (window as any).__nativeTest;
    mock.installedAgentCliBlocked = true;
    const now = Date.now;
    mock.restoreClock = () => {
      Date.now = now;
    };
    Date.now = () => now() + 5 * 60 * 1000 + 1;
  });
  dialog = await openAgents(page);
  expect(await dialog.getByRole("radio").count()).toBe(2);
  await expect(dialog.getByRole("status")).toHaveText(
    "Refreshing installed agents…",
  );
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).__nativeTest.installedAgentCliCalls.length,
      ),
    )
    .toBe(2);
  await page.evaluate(() => (window as any).__nativeTest.restoreClock());
  await dialog.getByLabel("Number of terminals").fill("1");
  await dialog.getByRole("button", { name: "Launch agents" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "Cursor CLI", exact: true }),
  ).toBeVisible();
  await releaseScans(page);
});
