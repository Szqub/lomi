import { test, expect } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { mockRemoteAccount } from "./remote-auth";

test("Remote uses workspace sharing and keeps browser revocation without manual setup", async ({
  page,
}, testInfo) => {
  await mockDesktop(page, false);
  await mockRemoteAccount(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const state = {
      qualified: true,
      enabled: true,
      online: true,
      message: null,
      domainEpoch: null,
      workspaces: [
        { id: "workspace", shared: true, online: true, message: null },
      ],
      grants: [
        {
          id: "grant",
          sessionIds: ["terminal"],
          permissions: "control",
          expiresAt: 9999999999,
          revoked: false,
        },
      ],
    };
    desktop.__remoteInvoke = async (command: string) => {
      if (command === "remote_revoke_grant") state.grants[0].revoked = true;
      return state;
    };
  });
  await page.goto("/?window=settings&page=remote");
  await expect(
    page.getByRole("heading", { name: "Remote", exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/Right-click a workspace/)).toBeVisible();
  await expect(page.getByRole("textbox")).toHaveCount(0);
  await expect(page.getByRole("checkbox")).toHaveCount(0);
  await page.getByRole("button", { name: "Revoke access" }).click();
  await expect(page.getByText("No browsers have access.")).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("remote-workspace-settings.png"),
    fullPage: true,
  });
});
