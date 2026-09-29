import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

type TestUsageWindow = {
  label: string;
  remainingPercent: number | null;
  used: number | null;
  limit: number | null;
  unit: string | null;
  resetsAt: number | null;
};

function context(cli: string, pid: number) {
  return {
    cwd: "/project",
    foregroundProgram: cli,
    titleCli: { cli, pid },
  };
}

function entry(
  id: string,
  cli: string,
  pid: number,
  windows: TestUsageWindow[],
  options: {
    status?: string;
    updatedAt?: number | null;
    retryAt?: number | null;
    message?: string | null;
    source?: string | null;
  } = {},
) {
  return {
    id,
    process: { cli, pid },
    status: options.status ?? "ready",
    windows,
    updatedAt: options.updatedAt === undefined ? Date.now() : options.updatedAt,
    retryAt: options.retryAt ?? null,
    message: options.message ?? null,
    source: options.source ?? null,
  };
}

async function setUsage(
  page: Page,
  contexts: Record<string, ReturnType<typeof context>>,
  entries: ReturnType<typeof entry>[],
) {
  await page.evaluate(
    ({ contexts, entries }) => {
      const native = (window as any).__nativeTest;
      native.terminalContexts = contexts;
      native.cliUsageEntries = entries;
    },
    { contexts, entries },
  );
}

async function usageCalls(page: Page) {
  return page.evaluate(() => (window as any).__nativeTest.cliUsageCalls);
}

test("remaining usage updates on the timer and focus without opening the menu", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.clock.install();
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const now = await page.evaluate(() => Date.now());
  await page.clock.pauseAt(new Date(now + 1000));
  const contexts = { "live-session": context("codex", 299) };
  const quota = (remainingPercent: number) =>
    entry("live-session", "codex", 299, [
      {
        label: "5-hour",
        remainingPercent,
        used: null,
        limit: null,
        unit: null,
        resetsAt: null,
      },
    ]);
  await setUsage(page, contexts, [quota(80)]);
  await page.clock.runFor(1000);
  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toHaveText("80%");

  const initialCalls = (await usageCalls(page)).length;
  await setUsage(page, contexts, [quota(60)]);
  await page.clock.runFor(5000);
  await expect(trigger).toHaveText("80%");
  expect((await usageCalls(page)).length).toBe(initialCalls);
  await page.clock.runFor(10_000);
  await expect(trigger).toHaveText("60%");
  expect((await usageCalls(page)).length).toBeGreaterThan(initialCalls);
  await expect(page.locator(".agent-usage-menu")).toHaveCount(0);

  await setUsage(page, contexts, [quota(40)]);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(trigger).toHaveText("40%");
  await setUsage(page, {}, []);
  await page.clock.runFor(1000);
  await expect(trigger).toHaveCount(0);
  const stoppedCalls = (await usageCalls(page)).length;
  await page.clock.runFor(30_000);
  expect((await usageCalls(page)).length).toBe(stoppedCalls);
});

test("the titlebar summarizes the tightest window and details stale multi-agent usage", async ({
  page,
}) => {
  await page.setViewportSize({ width: 920, height: 680 });
  await page.emulateMedia({ colorScheme: "light" });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "start_terminal",
          ).length,
      ),
    )
    .toBe(1);
  const startsBefore = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ).length,
  );

  const reset = Date.now() + 12 * 60 * 60 * 1000;
  await setUsage(
    page,
    {
      "codex-session": context("codex", 301),
      "claude-session": context("claude", 302),
    },
    [
      entry(
        "codex-session",
        "codex",
        301,
        [
          {
            label: "5-hour",
            remainingPercent: 65.25,
            used: 6_950,
            limit: 20_000,
            unit: "tokens",
            resetsAt: reset,
          },
          {
            label: "Weekly",
            remainingPercent: 12.4,
            used: 876,
            limit: 1_000,
            unit: "requests",
            resetsAt: reset,
          },
        ],
        {
          status: "rate-limited",
          updatedAt: Date.now() - 60_000,
          retryAt: Date.now() + 60_000,
          message:
            "Usage refresh is limited. Try again after the current window.",
          source: "oauth",
        },
      ),
      entry(
        "claude-session",
        "claude",
        302,
        [
          {
            label: "Current session",
            remainingPercent: 42.5,
            used: 575,
            limit: 1_000,
            unit: "messages",
            resetsAt: reset,
          },
        ],
        { source: "https://usage.example/account?secret=hidden" },
      ),
    ],
  );

  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toContainText("12.4%");
  await expect(trigger).toHaveAttribute(
    "aria-label",
    /remaining across 2 agents, stale data/,
  );
  await expect(trigger).toHaveAttribute(
    "title",
    /Codex · Weekly: 12.4% remaining · stale data/,
  );
  expect(
    await trigger.evaluate(
      (element) =>
        element.compareDocumentPosition(
          document.querySelector(".titlebar-account-control")!,
        ) & Node.DOCUMENT_POSITION_FOLLOWING,
    ),
  ).toBeTruthy();

  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("group")).toHaveCount(2);
  await expect(menu).toContainText("5-hour");
  await expect(menu).toContainText("Weekly");
  const weeklyQuota = menu.getByRole("progressbar", {
    name: "Weekly quota: 12.4% remaining",
  });
  await expect(weeklyQuota).toBeVisible();
  await expect(weeklyQuota).toHaveJSProperty("value", 12.4);
  await expect(menu).toContainText("876 requests used / 1,000 requests");
  await expect(menu).toContainText("Source · Connected account");
  await expect(menu).toContainText("Usage refresh is limited.");
  await expect(menu).not.toContainText("usage.example");
  await page.screenshot({ path: "/tmp/lomi-agent-usage-menu-light.png" });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.appearance),
    )
    .toBe("dark");
  await page.screenshot({ path: "/tmp/lomi-agent-usage-menu-dark.png" });

  const refresh = menu.getByRole("menuitem", { name: "Refresh", exact: true });
  const beforeRefresh = await usageCalls(page);
  await refresh.click();
  await expect
    .poll(async () => (await usageCalls(page)).length)
    .toBeGreaterThan(beforeRefresh.length);
  await expect
    .poll(async () => (await usageCalls(page)).at(-1)?.force)
    .toBe(true);

  const protectedCalls = await page.evaluate(() =>
    (window as any).__nativeTest.calls.filter(
      (call: any) =>
        call.command === "write_terminal" ||
        call.command === "enable_cli_integration" ||
        call.command === "enable_cli_titles",
    ),
  );
  const startsAfter = await page.evaluate(
    () =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ).length,
  );
  expect(protectedCalls).toHaveLength(0);
  expect(startsAfter).toBe(startsBefore);
});

test("each CLI quota bar and titlebar use the same remaining percentage", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();

  const agents = [
    { id: "codex-quota", cli: "codex", pid: 601, label: "Codex window" },
    {
      id: "claude-quota",
      cli: "claude",
      pid: 602,
      label: "Claude window",
    },
    {
      id: "cursor-quota",
      cli: "cursor",
      pid: 603,
      label: "Cursor window",
    },
    { id: "kimi-quota", cli: "kimi", pid: 604, label: "Kimi window" },
    {
      id: "agy-quota",
      cli: "agy",
      pid: 605,
      label: "Antigravity window",
    },
  ];
  const contexts = Object.fromEntries(
    agents.map(({ id, cli, pid }) => [id, context(cli, pid)]),
  );
  const quotaEntries = (remainingPercent: number) =>
    agents.map(({ id, cli, pid, label }) =>
      entry(id, cli, pid, [
        {
          label,
          remainingPercent,
          used: null,
          limit: null,
          unit: null,
          resetsAt: null,
        },
      ]),
    );

  await setUsage(page, contexts, quotaEntries(100));
  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger.locator(".agent-usage-trigger-value")).toHaveText(
    "100%",
  );
  await expect(trigger).toHaveAttribute(
    "aria-label",
    "CLI account usage, 100% remaining across 5 agents",
  );
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");

  const expectRemaining = async (remainingPercent: number) => {
    await expect(trigger.locator(".agent-usage-trigger-value")).toHaveText(
      `${remainingPercent}%`,
    );
    await expect(trigger).toHaveAttribute(
      "title",
      new RegExp(`: ${remainingPercent}% remaining`),
    );
    await expect(menu.getByRole("progressbar")).toHaveCount(agents.length);
    for (const agent of agents) {
      const quota = menu.getByRole("progressbar", {
        name: `${agent.label} quota: ${remainingPercent}% remaining`,
      });
      await expect(quota).toHaveJSProperty("value", remainingPercent);
    }
  };

  await expectRemaining(100);
  for (const remainingPercent of [75, 0]) {
    await setUsage(page, contexts, quotaEntries(remainingPercent));
    const callsBefore = (await usageCalls(page)).length;
    await menu.getByRole("menuitem", { name: "Refresh", exact: true }).click();
    await expect
      .poll(async () => (await usageCalls(page)).length)
      .toBeGreaterThan(callsBefore);
    await expectRemaining(remainingPercent);
  }
});

test("keyboard, outside click, and agent removal dismiss the anchored usage menu", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await setUsage(page, { "first-session": context("codex", 401) }, [
    entry("first-session", "codex", 401, [
      {
        label: "Daily",
        remainingPercent: 44,
        used: 56,
        limit: 100,
        unit: "requests",
        resetsAt: Date.now() + 60_000,
      },
    ]),
  ]);

  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toBeVisible();
  await trigger.focus();
  await page.keyboard.press("ArrowDown");
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Refresh" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(trigger).toBeFocused();

  await trigger.click({ button: "right" });
  await expect(menu).toBeVisible();
  await page.setViewportSize({ width: 800, height: 600 });
  await expect(menu).toHaveCount(0);
  await trigger.click();
  await expect(menu).toBeVisible();
  await page.locator(".work-area").click({ position: { x: 380, y: 300 } });
  await expect(menu).toHaveCount(0);

  await trigger.click();
  await expect(menu).toBeVisible();
  await setUsage(page, {}, []);
  await expect(trigger).toHaveCount(0);
  await expect(menu).toHaveCount(0);

  await setUsage(page, { "next-session": context("claude", 402) }, [
    entry("next-session", "claude", 402, [
      {
        label: "Session",
        remainingPercent: 72,
        used: 28,
        limit: 100,
        unit: "messages",
        resetsAt: null,
      },
    ]),
  ]);
  await expect(trigger).toBeVisible();
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  await expect(menu).toHaveCount(0);
});

test("unknown quotas show an em dash and a useful status without false stale warning", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await setUsage(page, { "codex-session": context("codex", 451) }, [
    entry(
      "codex-session",
      "codex",
      451,
      [
        {
          label: "Account quota",
          remainingPercent: null,
          used: null,
          limit: null,
          unit: null,
          resetsAt: null,
        },
      ],
      {
        status: "unsupported",
        updatedAt: null,
        message: "This account is using a quota mode the CLI cannot read.",
      },
    ),
  ]);

  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toContainText("—");
  await expect(trigger).toHaveAttribute("title", /Usage is unavailable/);
  await expect(trigger.locator(".agent-usage-stale-dot")).toHaveCount(0);
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toContainText("Usage unavailable");
  await expect(menu).toContainText("This account is using a quota mode");
  await expect(menu).toContainText("Quota unknown");
  await expect(menu.getByRole("progressbar")).toHaveCount(0);
  await expect(menu.locator(".agent-usage-status svg")).toHaveCount(0);
});

test("late usage results are ignored after the running agent changes", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await page.evaluate(() => {
    (window as any).__nativeTest.cliUsageDelay = 350;
  });
  await setUsage(page, { "codex-session": context("codex", 501) }, [
    entry("codex-session", "codex", 501, [
      {
        label: "Old account",
        remainingPercent: 4,
        used: 96,
        limit: 100,
        unit: "requests",
        resetsAt: null,
      },
    ]),
    entry("claude-session", "claude", 502, [
      {
        label: "New account",
        remainingPercent: 37.4,
        used: 626,
        limit: 1_000,
        unit: "tokens",
        resetsAt: null,
      },
    ]),
  ]);
  await expect.poll(async () => (await usageCalls(page)).length).toBe(1);
  await setUsage(page, { "claude-session": context("claude", 502) }, [
    entry("codex-session", "codex", 501, [
      {
        label: "Old account",
        remainingPercent: 4,
        used: 96,
        limit: 100,
        unit: "requests",
        resetsAt: null,
      },
    ]),
    entry("claude-session", "claude", 502, [
      {
        label: "New account",
        remainingPercent: 37.4,
        used: 626,
        limit: 1_000,
        unit: "tokens",
        resetsAt: null,
      },
    ]),
  ]);

  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toHaveText("37.4%");
  const calls = await usageCalls(page);
  expect(calls).toHaveLength(2);
  expect(calls[0].targets).toEqual([
    { id: "codex-session", process: { cli: "codex", pid: 501 } },
  ]);
  expect(calls[1].targets).toEqual([
    { id: "claude-session", process: { cli: "claude", pid: 502 } },
  ]);
});

test("the details list can be scrolled with the keyboard when many agents are active", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const contexts = Object.fromEntries(
    Array.from({ length: 12 }, (_, index) => [
      `agent-session-${index + 1}`,
      context("codex", 650 + index),
    ]),
  );
  const entries = Object.entries(contexts).map(([id, value], index) =>
    entry(id, "codex", value.titleCli.pid, [
      {
        label: `Window ${index + 1}`,
        remainingPercent: 80 - index,
        used: 20 + index,
        limit: 100,
        unit: "requests",
        resetsAt: null,
      },
    ]),
  );
  await setUsage(page, contexts, entries);

  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toContainText("69%");
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  const list = menu.getByRole("region", { name: "Usage details by agent" });
  await expect(list).toBeVisible();
  await page.keyboard.press("Shift+Tab");
  await expect(list).toBeFocused();
  const before = await list.evaluate((element) => element.scrollTop);
  await page.keyboard.press("End");
  await expect
    .poll(() => list.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(before);
  await expect(
    menu.getByRole("group", { name: "Codex usage 12" }),
  ).toBeInViewport();
});

test("usage details stay within a compact titlebar at reduced zoom", async ({
  page,
}) => {
  await page.setViewportSize({ width: 440, height: 300 });
  await mockDesktop(page, false);
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  await setUsage(page, { "codex-session": context("codex", 601) }, [
    entry("codex-session", "codex", 601, [
      {
        label: "5-hour",
        remainingPercent: 0.4,
        used: 999.6,
        limit: 1_000,
        unit: "tokens",
        resetsAt: Date.now() + 24 * 60 * 60 * 1000,
      },
    ]),
  ]);
  const trigger = page.locator(".agent-usage-trigger");
  await expect(trigger).toContainText("0.4%");
  await page.keyboard.press("Control+Minus");
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.zoom))
    .toBe(0.9);
  await trigger.click();
  const menu = page.locator(".agent-usage-menu");
  await expect(menu).toBeVisible();
  const bounds = await menu.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(8);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(432);
  await page.screenshot({ path: "/tmp/lomi-agent-usage-menu-compact.png" });
});
