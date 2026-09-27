import { expect, test, type Page } from "@playwright/test";
import { unavailableState, type AuthState } from "../../src/auth/model";
import { mockDesktop } from "./desktop";

const signedOut: AuthState = {
  ...unavailableState,
  revision: 1,
  status: "signed-out",
};

const signedIn: AuthState = {
  ...signedOut,
  revision: 10,
  status: "signed-in",
  storage: "persistent",
  user: {
    id: "user-1",
    displayName: "Lomi User",
    email: "user@example.test",
    githubLogin: "lomi-user",
    status: "active",
  },
  session: { id: "session-1", expiresAt: "2026-10-01T12:00:00Z" },
};

const avatarUrl = "https://avatars.githubusercontent.com/lomi-user?s=64";
const fakeAvatar =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="#3269a8"/></svg>';

async function mockMainWindow(
  page: Page,
  {
    state = signedOut,
    welcome = false,
    deferInitialRead = false,
    deferBeginLogin = false,
  }: {
    state?: AuthState;
    welcome?: boolean;
    deferInitialRead?: boolean;
    deferBeginLogin?: boolean;
  } = {},
) {
  await mockDesktop(page, false, welcome ? null : undefined);
  await page.addInitScript(
    ({ initialState, holdInitialRead, deferBeginLogin }) => {
      const desktop = window as any;
      desktop.__authTest = {
        state: structuredClone(initialState),
        calls: [] as { command: string; args: unknown }[],
        holdInitialRead,
        holdBeginLogin: deferBeginLogin,
        initialReadStarted: false,
        initialReadHeld: false,
        resolveInitialRead: null as null | (() => void),
        beginLoginStarted: false,
        beginLoginHeld: false,
        resolveBeginLogin: null as null | (() => Promise<void>),
        failCommand: null as string | null,
        remoteRevocationConfirmed: true,
      };
      desktop.__authInvoke = async (command: string, args: unknown = {}) => {
        const fixture = desktop.__authTest;
        fixture.calls.push({ command, args: structuredClone(args) });
        if (command === fixture.failCommand)
          throw new Error("The account portal could not be opened.");
        if (command === "auth_sign_out") {
          fixture.state = {
            ...fixture.state,
            revision: fixture.state.revision + 1,
            status: "signed-out",
            user: null,
            session: null,
            storage: null,
            attempt: null,
            remoteRevocationConfirmed: fixture.remoteRevocationConfirmed,
          };
        }
        if (command === "auth_begin_login") {
          const started = {
            ...fixture.state,
            revision: fixture.state.revision + 1,
            status: "authorizing",
            storage: (args as { storage: "persistent" | "session" }).storage,
            attempt: {
              id: "attempt-1",
              expiresAt: "2030-01-01T00:00:00.000Z",
            },
          };
          if (fixture.holdBeginLogin && !fixture.beginLoginHeld) {
            fixture.beginLoginStarted = true;
            fixture.beginLoginHeld = true;
            return new Promise((resolve) => {
              fixture.resolveBeginLogin = async () => {
                fixture.state = started;
                await desktop.__nativeTest.emitEvent(
                  "auth-state-changed",
                  structuredClone(started),
                );
                resolve(structuredClone(started));
              };
            });
          }
          fixture.state = started;
        }
        if (
          command === "auth_get_state" &&
          fixture.holdInitialRead &&
          !fixture.initialReadHeld
        ) {
          fixture.initialReadStarted = true;
          fixture.initialReadHeld = true;
          const captured = structuredClone(fixture.state);
          return new Promise((resolve) => {
            fixture.resolveInitialRead = () => resolve(captured);
          });
        }
        return structuredClone(fixture.state);
      };
    },
    {
      initialState: state,
      holdInitialRead: deferInitialRead,
      deferBeginLogin,
    },
  );
  await page.goto("/");
}

function accountButton(page: Page) {
  return page.locator('.titlebar button[aria-label*="account" i]');
}

async function emitAuthState(page: Page, state: AuthState) {
  await page.evaluate(async (payload) => {
    await (window as any).__nativeTest.emitEvent("auth-state-changed", payload);
  }, state);
}

async function assertVerificationLaunch(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter(
            (call: any) =>
              call.command === "auth_begin_login" ||
              call.command === "auth_open_verification",
          )
          .map((call: any) => ({ command: call.command, args: call.args })),
      ),
    )
    .toEqual([
      { command: "auth_begin_login", args: { storage: "persistent" } },
      { command: "auth_open_verification", args: { attemptId: "attempt-1" } },
    ]);
}

for (const destination of ["welcome", "workspace"] as const) {
  test(`Sign In starts GitHub login from the ${destination}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 800, height: 600 });
    await mockMainWindow(page, { welcome: destination === "welcome" });

    const signIn = page.locator(".titlebar").getByRole("button", {
      name: "Sign In",
      exact: true,
    });
    await expect(signIn).toBeVisible();
    if (destination === "welcome") {
      await page.locator(".titlebar").screenshot({
        path: "/tmp/lomi-titlebar-sign-in.png",
      });
    }
    await signIn.click();
    await assertVerificationLaunch(page);
  });
}

test("a pending sign-in survives opening a project from the welcome screen", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await mockMainWindow(page, { welcome: true, deferBeginLogin: true });

  await page
    .locator(".titlebar")
    .getByRole("button", { name: "Sign In", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__authTest.beginLoginStarted),
    )
    .toBe(true);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "auth_open_verification",
          ).length,
      ),
    )
    .toBe(0);

  await page
    .getByRole("button", { name: "Open folder or repository", exact: true })
    .click();
  await expect(page.locator(".project-switcher")).toHaveText("chosen folder");
  await page.evaluate(() => (window as any).__authTest.resolveBeginLogin());
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "auth_open_verification")
          .map((call: any) => call.args),
      ),
    )
    .toEqual([{ attemptId: "attempt-1" }]);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__nativeTest.calls.filter(
            (call: any) => call.command === "auth_begin_login",
          ).length,
      ),
    )
    .toBe(1);
});

test("an existing authorization attempt opens without starting another login", async ({
  page,
}) => {
  const existingAttempt: AuthState = {
    ...signedOut,
    revision: 4,
    status: "authorizing",
    storage: "persistent",
    attempt: {
      id: "attempt-existing",
      expiresAt: "2030-01-01T00:00:00.000Z",
    },
  };
  await mockMainWindow(page, { state: existingAttempt });

  const signIn = page
    .locator(".titlebar")
    .getByRole("button", { name: "Sign In", exact: true });
  await expect(signIn).toHaveAttribute("title", /Waiting for browser/);
  await signIn.click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter(
            (call: any) =>
              call.command === "auth_begin_login" ||
              call.command === "auth_open_verification",
          )
          .map((call: any) => ({ command: call.command, args: call.args })),
      ),
    )
    .toEqual([
      {
        command: "auth_open_verification",
        args: { attemptId: "attempt-existing" },
      },
    ]);
});

test("the newest auth event wins over the initial read and older events", async ({
  page,
}) => {
  await page.route("https://avatars.githubusercontent.com/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "image/svg+xml",
      body: fakeAvatar,
    }),
  );
  await mockMainWindow(page, { deferInitialRead: true });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__authTest.initialReadStarted),
    )
    .toBe(true);

  const registrationOrder = await page.evaluate(() => {
    const calls = (window as any).__nativeTest.calls as {
      command: string;
      args: { event?: string };
    }[];
    return {
      listener: calls.findIndex(
        (call) =>
          call.command === "plugin:event|listen" &&
          call.args.event === "auth-state-changed",
      ),
      initialRead: calls.findIndex((call) => call.command === "auth_get_state"),
    };
  });
  expect(registrationOrder.listener).toBeGreaterThanOrEqual(0);
  expect(registrationOrder.initialRead).toBeGreaterThan(
    registrationOrder.listener,
  );

  const newest = { ...signedIn, revision: 12 };
  await emitAuthState(page, newest);
  await page.evaluate(() => (window as any).__authTest.resolveInitialRead());

  const account = accountButton(page);
  const avatar = account.locator("img");
  await expect(account).toBeVisible();
  await expect(avatar).toHaveAttribute("src", avatarUrl);
  await emitAuthState(page, {
    ...signedOut,
    revision: 11,
  });
  await expect(avatar).toBeVisible();
  await expect(
    page.locator(".titlebar").getByRole("button", {
      name: "Sign In",
      exact: true,
    }),
  ).toHaveCount(0);

  await emitAuthState(page, { ...signedOut, revision: 13 });
  await expect(
    page.locator(".titlebar").getByRole("button", {
      name: "Sign In",
      exact: true,
    }),
  ).toBeVisible();
});

test("offline and checking account states retain the GitHub avatar", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.emulateMedia({ colorScheme: "light" });
  let avatarReferrer: string | null = null;
  await page.route("https://avatars.githubusercontent.com/**", (route) => {
    avatarReferrer = route.request().headers().referer ?? null;
    return route.fulfill({
      status: 200,
      contentType: "image/svg+xml",
      body: fakeAvatar,
    });
  });
  await mockMainWindow(page, { state: signedIn });

  const account = accountButton(page);
  const avatar = account.locator("img");
  await expect(account).toBeVisible();
  await expect(avatar).toHaveAttribute("src", avatarUrl);
  await expect
    .poll(() =>
      avatar.evaluate((image: HTMLImageElement) => image.naturalWidth),
    )
    .toBeGreaterThan(0);
  expect(avatarReferrer).toBeNull();
  await page.locator(".titlebar").screenshot({
    path: "/tmp/lomi-titlebar-account-light.png",
  });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.locator(".titlebar").screenshot({
    path: "/tmp/lomi-titlebar-account-dark.png",
  });

  await emitAuthState(page, { ...signedIn, revision: 11, status: "offline" });
  await expect(avatar).toBeVisible();
  await expect(avatar).toHaveAttribute("src", avatarUrl);
  await emitAuthState(page, { ...signedIn, revision: 12, status: "checking" });
  await expect(avatar).toBeVisible();
  await expect(avatar).toHaveAttribute("src", avatarUrl);
  await expect(
    page.locator(".titlebar").getByRole("button", {
      name: "Sign In",
      exact: true,
    }),
  ).toHaveCount(0);
});

test("a failed avatar falls back to initials and opens the account menu at 800px", async ({
  page,
}) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.route("https://avatars.githubusercontent.com/**", (route) =>
    route.fulfill({ status: 404, contentType: "text/plain", body: "missing" }),
  );
  await mockMainWindow(page, { state: signedIn });

  const account = accountButton(page);
  const settings = page.locator('.titlebar button[aria-label^="Settings"]');
  await expect(account).toBeVisible();
  await expect(settings).toHaveCount(0);
  await expect(account.getByText("LU", { exact: true })).toBeVisible();

  const layout = await page.evaluate(() => {
    const header = document.querySelector(".titlebar")!;
    const account = header.querySelector('button[aria-label*="account" i]')!;
    const rect = (element: Element) => {
      const { left, right, top, bottom } = element.getBoundingClientRect();
      return { left, right, top, bottom };
    };
    return {
      viewport: window.innerWidth,
      document: document.documentElement.scrollWidth,
      header: header.clientWidth,
      headerContent: header.scrollWidth,
      account: rect(account),
    };
  });
  expect(layout.viewport).toBe(800);
  expect(layout.document).toBeLessThanOrEqual(layout.viewport);
  expect(layout.headerContent).toBeLessThanOrEqual(layout.header);
  expect(layout.account.right).toBeLessThanOrEqual(layout.viewport);
  await page.locator(".titlebar").screenshot({
    path: "/tmp/lomi-titlebar-account-fallback.png",
  });

  await account.click();
  const menu = page.getByRole("menu", { name: "Account menu" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Settings",
    "Account settings",
    "Manage account",
    "Sign out",
  ]);
  const menuBounds = await menu.boundingBox();
  expect(menuBounds!.x + menuBounds!.width).toBeCloseTo(
    layout.account.right,
    0,
  );
  await page.screenshot({ path: "/tmp/lomi-account-menu-light.png" });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: "/tmp/lomi-account-menu-dark.png" });
  await menu.getByRole("menuitem", { name: "Settings", exact: true }).click();
  await expect(menu).toHaveCount(0);
  await account.click();
  await menu.getByRole("menuitem", { name: "Account settings" }).click();
  await expect(menu).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter((call: any) => call.command === "open_settings")
          .map((call: any) => call.args),
      ),
    )
    .toEqual([{}, { page: "account" }]);
});

test("account menu supports keyboard navigation, toggle, and outside dismissal", async ({
  page,
}) => {
  await mockMainWindow(page, {
    state: { ...signedIn, user: { ...signedIn.user!, githubLogin: null } },
    welcome: true,
  });
  const account = accountButton(page);
  const menu = page.getByRole("menu", { name: "Account menu" });
  await expect(account).toHaveAttribute("aria-haspopup", "menu");
  await account.focus();
  await page.keyboard.press("ArrowDown");
  await expect(account).toHaveAttribute("aria-expanded", "true");
  await expect(
    menu.getByRole("menuitem", { name: "Settings", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(
    menu.getByRole("menuitem", { name: "Account settings" }),
  ).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(
    menu.getByRole("menuitem", { name: "Manage account" }),
  ).toBeFocused();
  await page.keyboard.press("End");
  await expect(
    menu.getByRole("menuitem", { name: "Sign out", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(account).toBeFocused();
  await expect(account).toHaveAttribute("aria-expanded", "false");
  await account.click();
  await expect(menu).toBeVisible();
  await account.click();
  await expect(menu).toHaveCount(0);
  await account.click();
  await page.mouse.click(400, 400);
  await expect(menu).toHaveCount(0);
  await account.click({ button: "right" });
  await expect(menu).toBeVisible();
  await emitAuthState(page, { ...signedOut, revision: 11 });
  await expect(menu).toHaveCount(0);
});

test("Manage account uses the native portal action and reports launch failures", async ({
  page,
}) => {
  await mockMainWindow(page, { state: signedIn });
  const account = accountButton(page);
  await account.click();
  await page.getByRole("menuitem", { name: "Manage account" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).__authTest.calls.filter(
            (call: any) => call.command === "auth_open_account_portal",
          ).length,
      ),
    )
    .toBe(1);
  await expect(page.getByRole("menu", { name: "Account menu" })).toHaveCount(0);
  await page.evaluate(() => {
    (window as any).__authTest.failCommand = "auth_open_account_portal";
  });
  await account.click();
  await page.getByRole("menuitem", { name: "Manage account" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "The account portal could not be opened.",
  );
  await expect(account).toBeEnabled();
});

for (const offline of [false, true]) {
  test(`Sign out from the avatar clears the account${offline ? " while offline" : ""}`, async ({
    page,
  }) => {
    await mockMainWindow(page, {
      state: { ...signedIn, status: offline ? "offline" : "signed-in" },
    });
    if (offline)
      await page.evaluate(() => {
        (window as any).__authTest.remoteRevocationConfirmed = false;
      });
    await accountButton(page).click();
    await page.getByRole("menuitem", { name: "Sign out", exact: true }).click();
    await expect(
      page
        .locator(".titlebar")
        .getByRole("button", { name: "Sign In", exact: true }),
    ).toBeVisible();
    await expect(page.getByRole("menu", { name: "Account menu" })).toHaveCount(
      0,
    );
    expect(
      await page.evaluate(
        () =>
          (window as any).__authTest.calls.filter(
            (call: any) => call.command === "auth_sign_out",
          ).length,
      ),
    ).toBe(1);
    if (offline)
      await expect(page.getByRole("alert")).toContainText(
        "server could not confirm",
      );
  });
}
