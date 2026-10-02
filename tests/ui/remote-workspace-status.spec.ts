import { expect, test, type Page } from "@playwright/test";
import { active, addWorkspace, newSession } from "../../src/model";
import { mockDesktop } from "./desktop";
import { mockRemoteAccount } from "./remote-auth";

async function setup(page: Page, sidebar = false) {
  let session = addWorkspace(newSession(), "/project", "local:bash", "Other");
  const otherId = active(session)!.workspace.id;
  session = addWorkspace(session, "/project", "local:bash", "Current");
  const currentId = active(session)!.workspace.id;
  session.sidebar = sidebar ? "workspaces" : null;
  await mockDesktop(page, false, session);
  await mockRemoteAccount(page);
  await page.addInitScript(
    ({ currentId, otherId }) => {
      const desktop = window as any;
      desktop.__nativeTest.remoteState = {
        qualified: true,
        enabled: true,
        online: true,
        message: null,
        domainEpoch: null,
        workspaces: [
          { id: currentId, shared: true, online: true, message: null },
          { id: otherId, shared: false, online: true, message: null },
        ],
        grants: [],
      };
    },
    { currentId, otherId },
  );
  await page.goto("/");
  await expect(page.locator("footer .remote-workspace-status")).toBeVisible();
  return { currentId, otherId };
}

async function unshareCalls(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls.filter(
      (call: any) => call.command === "remote_share_workspace",
    ),
  );
}

test("footer follows the active shared workspace and remains visible offline", async ({
  page,
}, testInfo) => {
  await setup(page, true);
  const status = page.locator("footer .remote-workspace-status");
  await expect(status).toHaveAttribute("title", /Current.*online/);
  await page.getByRole("button", { name: /^Other \/project/ }).click();
  await expect(status).toHaveCount(0);
  await page.getByRole("button", { name: /^Current \/project/ }).click();
  await expect(status).toBeVisible();
  await page.evaluate(async () => {
    const mock = (window as any).__nativeTest;
    mock.remoteState.online = false;
    for (const workspace of mock.remoteState.workspaces) {
      workspace.online = false;
      workspace.message = "Connection lost";
    }
    await mock.emitEvent("lomi-remote-state", mock.remoteState);
  });
  await expect(status).toHaveAttribute("title", "Current: Connection lost");
  await expect(
    status.getByText("Shared remotely", { exact: true }),
  ).toBeVisible();
  await page.setViewportSize({ width: 800, height: 600 });
  await page.screenshot({ path: testInfo.outputPath("remote-footer-800.png") });
  await expect(
    page.getByRole("button", { name: "Stop sharing remotely" }),
  ).toBeInViewport();
});

test("cancel and Escape preserve sharing; confirmation unshares only the current workspace", async ({
  page,
}, testInfo) => {
  const { currentId } = await setup(page);
  const trigger = page.getByRole("button", { name: "Stop sharing remotely" });
  const dialog = page.getByRole("alertdialog");
  await trigger.click();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
  await expect(dialog).toHaveAccessibleDescription(
    /Current.*Remote access will end.*Local terminals will keep running/,
  );
  await page.screenshot({
    path: testInfo.outputPath("remote-stop-dialog.png"),
  });
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  expect(await unshareCalls(page)).toEqual([]);
  await trigger.click();
  await dialog
    .getByRole("button", { name: "Stop sharing", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toHaveCount(0);
  expect(await unshareCalls(page)).toEqual([
    {
      command: "remote_share_workspace",
      args: { workspaceId: currentId, shared: false },
    },
  ]);
  const closed = await page.evaluate(() =>
    (window as any).__nativeTest.calls.filter(
      (call: any) => call.command === "close_terminal",
    ),
  );
  expect(closed).toEqual([]);
  await expect(page.locator(".xterm-screen:visible")).toBeVisible();
});

test("pending confirmation blocks dismissal and failures stay in the dialog for retry", async ({
  page,
}) => {
  const { currentId } = await setup(page);
  await page.evaluate(() => {
    const desktop = window as any;
    desktop.__remoteInvoke = async (command: string, args: any) => {
      if (command === "remote_share_workspace") {
        await new Promise<void>((resolve) => {
          desktop.__finishRemoteStop = resolve;
        });
        if (!desktop.__retryRemoteStop)
          throw new Error("Could not stop remote access");
        desktop.__nativeTest.remoteState.workspaces.find(
          (w: any) => w.id === args.workspaceId,
        ).shared = false;
      }
      return JSON.parse(JSON.stringify(desktop.__nativeTest.remoteState));
    };
  });
  await page.getByRole("button", { name: "Stop sharing remotely" }).click();
  const dialog = page.getByRole("alertdialog");
  const confirm = dialog.getByRole("button", {
    name: /^(Stop sharing|Stopping…)$/,
  });
  await confirm.click();
  await expect(confirm).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await expect(
    dialog.getByRole("button", { name: "Close dialog" }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await page.evaluate(() => (window as any).__finishRemoteStop());
  await expect(dialog.getByRole("alert")).toHaveText(
    "Could not stop remote access",
  );
  await expect(confirm).toBeEnabled();
  await page.evaluate(() => {
    (window as any).__retryRemoteStop = true;
  });
  await confirm.click();
  await expect(confirm).toBeDisabled();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await page.evaluate(() => (window as any).__finishRemoteStop());
  await expect(dialog).toHaveCount(0);
  const calls = await unshareCalls(page);
  expect(calls).toHaveLength(2);
  expect(
    calls.every(
      (call: any) =>
        call.args.workspaceId === currentId && call.args.shared === false,
    ),
  ).toBe(true);
});

for (const failure of ["rejected", "unconfirmed"] as const) {
  test(`starting sharing keeps ${failure} requests visible and allows retry`, async ({
    page,
  }) => {
    const { otherId } = await setup(page, true);
    await page.evaluate((failure) => {
      const desktop = window as any;
      desktop.__remoteInvoke = async (command: string, args: any) => {
        if (command === "remote_share_workspace") {
          await new Promise<void>((resolve) => {
            desktop.__finishRemoteStart = resolve;
          });
          if (!desktop.__retryRemoteStart) {
            if (failure === "rejected")
              throw new Error("Could not start remote access");
          } else {
            desktop.__nativeTest.remoteState.workspaces.find(
              (w: any) => w.id === args.workspaceId,
            ).shared = true;
          }
        }
        return JSON.parse(JSON.stringify(desktop.__nativeTest.remoteState));
      };
    }, failure);
    await page.getByRole("button", { name: /^Other \/project/ }).click({
      button: "right",
    });
    await page.getByRole("menuitem", { name: "Share remotely" }).click();
    const dialog = page.getByRole("alertdialog", {
      name: "Share workspace remotely?",
    });
    const confirm = dialog.getByRole("button", {
      name: /^(Share remotely|Sharing…)$/,
    });
    await confirm.click();
    await expect(confirm).toBeDisabled();
    await expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeVisible();
    await expect.poll(() => unshareCalls(page)).toHaveLength(1);
    await page.evaluate(() => (window as any).__finishRemoteStart());
    await expect(dialog.getByRole("alert")).toHaveText(
      failure === "rejected"
        ? "Could not start remote access"
        : "Remote sharing was not confirmed. Try sharing the workspace again.",
    );
    await expect(confirm).toBeEnabled();
    await page.evaluate(() => {
      (window as any).__retryRemoteStart = true;
    });
    await confirm.click();
    await expect(confirm).toBeDisabled();
    await expect(dialog.getByRole("alert")).toHaveCount(0);
    await expect.poll(() => unshareCalls(page)).toHaveLength(2);
    await page.evaluate(() => (window as any).__finishRemoteStart());
    await expect(dialog).toHaveCount(0);
    expect(await unshareCalls(page)).toEqual([
      {
        command: "remote_share_workspace",
        args: { workspaceId: otherId, shared: true },
      },
      {
        command: "remote_share_workspace",
        args: { workspaceId: otherId, shared: true },
      },
    ]);
  });
}
