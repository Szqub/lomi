import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { unavailableState } from "../../src/auth/model";

function items(count: number) {
  return Array.from({ length: count }, (_, i) => ({
    id: `item-${i}`,
    kind: i % 2 ? "finished" : "attention",
    title: `Agent update ${i + 1}`,
    body: "Example project · Main workspace · Build task",
    createdAt: Date.now() - i * 60000,
    read: false,
  }));
}
async function setup(
  page: Page,
  count = 1,
  signedIn = false,
  welcome = false,
  holdLoad = false,
) {
  await mockDesktop(page, false, welcome ? null : undefined);
  await page.addInitScript(
    ({ count, signedIn, holdLoad, entries, unavailableState }) => {
      const desktop = window as any;
      if (!localStorage.getItem("test-notifications")) {
        desktop.__nativeTest.notifications = {
          revision: count,
          items: entries,
        };
        localStorage.setItem(
          "test-notifications",
          JSON.stringify(desktop.__nativeTest.notifications),
        );
      }
      desktop.__nativeTest.holdNotificationLoad = holdLoad;
      desktop.__authInvoke = async () => ({
        ...unavailableState,
        revision: 1,
        status: signedIn ? "signed-in" : "signed-out",
        user: signedIn
          ? {
              id: "user",
              displayName: "Example User",
              email: null,
              githubLogin: null,
              status: "active",
            }
          : null,
        session: signedIn ? { id: "session", expiresAt: "2030-01-01" } : null,
      });
    },
    { count, signedIn, holdLoad, entries: items(count), unavailableState },
  );
  await page.goto("/");
}
const account = (page: Page) =>
  page.locator(
    ".titlebar .titlebar-account-avatar, .titlebar .titlebar-account-menu-trigger",
  );
const inbox = (page: Page) =>
  page.getByRole("dialog", { name: "Notifications", exact: true });
async function open(page: Page) {
  await account(page).click();
  await page
    .getByRole("menuitem", { name: "Notifications", exact: true })
    .click();
  await expect(inbox(page)).toBeVisible();
}
async function incoming(page: Page, title = "New arrival") {
  await page.evaluate(async (title) => {
    const mock = (window as any).__nativeTest;
    mock.notifications.revision++;
    mock.notifications.items.unshift({
      id: `arrival-${mock.notifications.revision}`,
      kind: "attention",
      title,
      body: "Another workspace",
      createdAt: Date.now(),
      read: false,
    });
    localStorage.setItem(
      "test-notifications",
      JSON.stringify(mock.notifications),
    );
    await mock.emitEvent(
      "notifications-changed",
      structuredClone(mock.notifications),
    );
  }, title);
}

for (const count of [1, 9, 10])
  test(`avatar shows ${count > 9 ? "9+" : count} unread badge`, async ({
    page,
  }) => {
    await setup(page, count, true);
    await expect(page.locator(".titlebar-account-badge")).toHaveText(
      count > 9 ? "9+" : String(count),
    );
    await expect(account(page)).toHaveAccessibleName(
      `Open account menu for Example User, ${count} unread notifications`,
    );
  });

test("signed-out welcome keeps receipts silent and reveals count only in the menu", async ({
  page,
}, testInfo) => {
  await setup(page, 1, false, true);
  await expect(page.locator(".titlebar-account-badge")).toHaveCount(0);
  await incoming(page);
  await expect(inbox(page)).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await account(page).click();
  await expect(
    page
      .getByRole("menuitem", { name: "Notifications", exact: true })
      .locator(".notification-badge"),
  ).toHaveText("2");
  await page.screenshot({
    path: testInfo.outputPath("signed-out-notifications-menu.png"),
  });
  await page
    .getByRole("menuitem", { name: "Notifications", exact: true })
    .click();
  await expect(inbox(page)).toContainText("2 unread");
  await expect(inbox(page).locator(".notification-item").first()).toContainText(
    "New arrival",
  );
  await expect(
    inbox(page).getByRole("button", { name: "Close notifications" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(account(page)).toBeFocused();
});

test("explicit reads, dismissal and clear read persist across reload", async ({
  page,
}) => {
  await setup(page, 3, true);
  await open(page);
  await expect(inbox(page)).toContainText("3 unread");
  await inbox(page)
    .getByRole("button", { name: "Agent update 1, unread", exact: true })
    .click();
  await expect(inbox(page)).toContainText("2 unread");
  await inbox(page)
    .getByRole("button", { name: "Dismiss Agent update 2", exact: true })
    .click();
  await expect(inbox(page).locator(".notification-item")).toHaveCount(2);
  await inbox(page)
    .getByRole("button", { name: "Clear read notifications", exact: true })
    .click();
  await expect(inbox(page).locator(".notification-item")).toHaveCount(1);
  await page.reload();
  await open(page);
  await expect(inbox(page).locator(".notification-item")).toHaveCount(1);
  await inbox(page)
    .getByRole("button", { name: "Agent update 3, unread", exact: true })
    .click();
  await expect(page.locator(".titlebar-account-badge")).toHaveCount(0);
  await expect(inbox(page)).toContainText("All caught up");
});

test("a stale initial load cannot overwrite a newer event", async ({
  page,
}) => {
  await setup(page, 1, true, false, true);
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean((window as any).__nativeTest.resolveNotificationLoad),
      ),
    )
    .toBe(true);
  await incoming(page);
  await page.evaluate(() =>
    (window as any).__nativeTest.resolveNotificationLoad(),
  );
  await open(page);
  await expect(inbox(page)).toContainText("2 unread");
  await expect(inbox(page)).toContainText("New arrival");
});

test("reading an item preserves new arrivals and ignores its stale response", async ({
  page,
}) => {
  await setup(page, 1, true);
  await open(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.holdNotificationMutation = true;
  });
  await inbox(page)
    .getByRole("button", { name: "Agent update 1, unread", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean((window as any).__nativeTest.resolveNotificationMutation),
      ),
    )
    .toBe(true);
  await incoming(page);
  await expect(inbox(page)).toContainText("1 unread");
  await page.evaluate(() =>
    (window as any).__nativeTest.resolveNotificationMutation(),
  );
  await expect(
    inbox(page).getByRole("button", {
      name: "New arrival, unread",
      exact: true,
    }),
  ).toBeEnabled();
  await expect(inbox(page)).toContainText("1 unread");
});

test("mutation failure keeps items and reports an inline retry", async ({
  page,
}) => {
  await setup(page, 1, true);
  await open(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.failNotificationCommand =
      "dismiss_notification";
  });
  await inbox(page)
    .getByRole("button", { name: "Dismiss Agent update 1", exact: true })
    .click();
  await expect(inbox(page).getByRole("status")).toContainText(
    "Notification storage is unavailable",
  );
  await expect(inbox(page).locator(".notification-item")).toHaveCount(1);
  await page.evaluate(() => {
    (window as any).__nativeTest.failNotificationCommand = null;
  });
  await inbox(page).getByRole("button", { name: "Retry", exact: true }).click();
  await expect(inbox(page).getByRole("status")).toHaveCount(0);
  await expect(inbox(page)).toContainText("1 unread");
});

for (const theme of ["dark", "light"])
  test(`long ${theme} inbox clamps to a small viewport and cycles keyboard focus`, async ({
    page,
  }, testInfo) => {
    await page.emulateMedia({ colorScheme: theme });
    await page.setViewportSize({ width: 380, height: 420 });
    await setup(page, 25, true);
    await open(page);
    const bounds = await inbox(page).boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(8);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(372);
    expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(412);
    const list = inbox(page).getByRole("region", { name: "Notification list" });
    expect(
      await list.evaluate(
        (element) => element.scrollHeight > element.clientHeight,
      ),
    ).toBe(true);
    await page.keyboard.press("Shift+Tab");
    await expect(
      inbox(page).getByRole("button", {
        name: "Clear read notifications",
        exact: true,
      }),
    ).not.toBeFocused();
    await expect(
      inbox(page).getByRole("button", {
        name: "Dismiss Agent update 25",
        exact: true,
      }),
    ).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(
      inbox(page).getByRole("button", { name: "Close notifications" }),
    ).toBeFocused();
    await list.evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.screenshot({
      path: testInfo.outputPath(`notifications-${theme}.png`),
    });
    await page.keyboard.press("Escape");
    await expect(account(page)).toBeFocused();
    await open(page);
    await page.mouse.click(8, 400);
    await expect(inbox(page)).toHaveCount(0);
    await open(page);
    await page.setViewportSize({ width: 400, height: 420 });
    await expect(inbox(page)).toHaveCount(0);
    await open(page);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await expect(inbox(page)).toHaveCount(0);
  });

test("passive load failures stay inline and retry resubscribes", async ({
  page,
}) => {
  await mockDesktop(page, false);
  await page.addInitScript(() => {
    (window as any).__nativeTest.failNotificationCommand = "load_notifications";
  });
  await page.goto("/");
  await expect(inbox(page)).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await open(page);
  await expect(inbox(page).getByRole("status")).toContainText(
    "Could not load notifications",
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.failNotificationCommand = null;
  });
  await inbox(page).getByRole("button", { name: "Retry", exact: true }).click();
  await expect(inbox(page)).toContainText("No notifications yet");
  await expect(inbox(page).getByRole("status")).toHaveCount(0);
  await incoming(page);
  await expect(inbox(page)).toContainText("New arrival");
});

test("a passive receipt error arriving during initial load remains visible", async ({
  page,
}) => {
  await setup(page, 1, false, false, true);
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean((window as any).__nativeTest.resolveNotificationLoad),
      ),
    )
    .toBe(true);
  await page.evaluate(async () => {
    const { reportNotificationError } = await import("/src/notifications.ts");
    reportNotificationError("Could not save notification: Disk is full");
    (window as any).__nativeTest.resolveNotificationLoad();
  });
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(inbox(page)).toHaveCount(0);
  await open(page);
  await expect(inbox(page).getByRole("status")).toContainText("Disk is full");
});

test("inbox keyboard focus protects underlying terminal panes from shortcuts", async ({
  page,
}) => {
  await setup(page, 1, true);
  await expect(page.locator(".terminal-host")).toHaveCount(1);
  await open(page);
  await page.keyboard.press("Control+d");
  await expect(page.locator(".terminal-host")).toHaveCount(1);
  await page.keyboard.press("Control+w");
  await expect(page.locator(".terminal-host")).toHaveCount(1);
  await expect(inbox(page)).toBeVisible();
  await expect(
    inbox(page).getByRole("button", { name: "Close notifications" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await page.locator(".terminal-host textarea").focus();
  await page.keyboard.press("Control+d");
  await expect(page.locator(".terminal-host")).toHaveCount(2);
});

test("an older retry cannot clear a newer mutation failure", async ({
  page,
}) => {
  await setup(page, 1, true);
  await open(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.failNotificationCommand =
      "dismiss_notification";
  });
  const dismiss = inbox(page).getByRole("button", {
    name: "Dismiss Agent update 1",
    exact: true,
  });
  await dismiss.click();
  await expect(inbox(page).getByRole("status")).toContainText(
    "Could not update notifications",
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.holdNotificationLoad = true;
  });
  await inbox(page).getByRole("button", { name: "Retry", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean((window as any).__nativeTest.resolveNotificationLoad),
      ),
    )
    .toBe(true);
  await dismiss.click();
  await expect(dismiss).toBeEnabled();
  await page.evaluate(() => {
    (window as any).__nativeTest.resolveNotificationLoad();
  });
  await expect(inbox(page).getByRole("status")).toContainText(
    "Could not update notifications",
  );
  await expect(inbox(page).locator(".notification-item")).toHaveCount(1);
});

for (const theme of ["dark", "light"] as const)
  for (const width of [1440, 380])
    test(`agent row fixture ${theme} at ${width}px keeps context and reveals dismiss actions`, async ({
      page,
    }, testInfo) => {
      await page.emulateMedia({ colorScheme: theme });
      await page.setViewportSize({ width, height: width === 380 ? 560 : 900 });
      await setup(page, 0, true);
      await page.evaluate(async () => {
        const mock = (window as any).__nativeTest;
        mock.notifications = {
          revision: 10,
          items: [
            {
              id: "codex",
              agent: "codex",
              kind: "finished",
              title: "Codex finished responding",
              body: "Lomi · Main workspace · Build task",
              createdAt: Date.now(),
              read: false,
            },
            {
              id: "claude",
              agent: "claude",
              kind: "attention",
              title: "Claude Code needs your input",
              body: "Example project · Review workspace",
              createdAt: Date.now() - 60000,
              read: false,
            },
            {
              id: "long",
              kind: "attention",
              title:
                "A very long agent notification title with aLongUnbrokenTaskIdentifierThatMustWrapWithoutOverflowingTheCompactNotificationPanel",
              body: "Stored workspace context stays discoverable",
              createdAt: Date.now() - 7200000,
              read: true,
            },
          ],
        };
        await mock.emitEvent(
          "notifications-changed",
          structuredClone(mock.notifications),
        );
      });
      await open(page);
      const rows = inbox(page).locator(".notification-item");
      await expect(rows.nth(0)).toContainText("Notification from Codex");
      await expect(rows.nth(1)).toContainText("Notification from Claude Code");
      await expect(rows.nth(2)).toContainText("Notification from terminal");
      await expect(rows.nth(0).locator("time")).toHaveText("Just now");
      await expect(rows.nth(1).locator("time")).toHaveText("1m ago");
      await expect(rows.nth(0).getByRole("button").first()).toHaveAttribute(
        "title",
        "Lomi · Main workspace · Build task",
      );
      await expect(
        rows.nth(0).getByRole("button").first(),
      ).toHaveAccessibleDescription(
        "Notification from Codex. Finished. Just now. Lomi · Main workspace · Build task",
      );
      await expect(
        inbox(page).getByRole("button", { name: "Mark as read", exact: true }),
      ).toHaveCount(0);
      expect(
        await inbox(page).evaluate(
          (element) => element.scrollWidth <= element.clientWidth,
        ),
      ).toBe(true);
      const dismiss = rows.nth(0).getByRole("button", {
        name: "Dismiss Codex finished responding",
        exact: true,
      });
      await page.mouse.move(0, 0);
      await expect(dismiss).toHaveCSS("opacity", "0");
      await rows.nth(0).hover();
      await expect(dismiss).toHaveCSS("opacity", "1");
      await page.mouse.move(0, 0);
      await inbox(page)
        .getByRole("button", { name: "Close notifications" })
        .focus();
      await page.keyboard.press("Tab");
      await page.keyboard.press("Tab");
      await expect(rows.nth(0).getByRole("button").first()).toBeFocused();
      await expect(rows.nth(0).locator(".notification-context")).toBeVisible();
      await expect(rows.nth(0).locator(".notification-context")).toContainText(
        "Lomi · Main workspace · Build task",
      );
      await expect(dismiss).toHaveCSS("opacity", "1");
      await page.keyboard.press("Tab");
      await expect(dismiss).toBeFocused();
      await page.screenshot({
        path: testInfo.outputPath(`agent-rows-${theme}-${width}.png`),
      });
      await page.keyboard.press("Enter");
      await expect(rows).toHaveCount(2);
      await expect(
        inbox(page).getByRole("button", { name: "Close notifications" }),
      ).toBeFocused();
    });

test("relative time updates while the inbox is open without reading the entry", async ({
  page,
}) => {
  await page.clock.install();
  await setup(page, 1, true);
  await open(page);
  await expect(inbox(page).locator("time")).toHaveText("Just now");
  await page.clock.fastForward(60_000);
  await expect(inbox(page).locator("time")).toHaveText("1m ago");
  await expect(inbox(page)).toContainText("1 unread");
});

test.describe("touch notification actions", () => {
  test.use({ hasTouch: true });
  test("dismiss stays visible and reachable without hover", async ({
    page,
  }) => {
    await setup(page, 1, true);
    await open(page);
    await expect(inbox(page).locator(".notification-context")).toBeVisible();
    await expect(inbox(page).locator(".notification-context")).toContainText(
      "Example project · Main workspace · Build task",
    );
    const dismiss = inbox(page).getByRole("button", {
      name: "Dismiss Agent update 1",
      exact: true,
    });
    await expect(dismiss).toHaveCSS("opacity", "1");
    await dismiss.tap();
    await expect(inbox(page)).toContainText("No notifications yet");
  });
});
