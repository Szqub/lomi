import { expect, test } from "@playwright/test";
import { active, addWorkspace, newSession, splitPane } from "../../src/model";
import { mockDesktop } from "./desktop";

test("workspace appearance saves on an inactive workspace, supports images and survives reload", async ({
  page,
}, testInfo) => {
  let session = addWorkspace(newSession(), "/voice", "local:bash", "Voice");
  session = addWorkspace(session, "/bench", "local:bash", "Bench");
  session.sidebar = "workspaces";
  await mockDesktop(page, false, session);
  await page.route(
    new URL("/", testInfo.project.use.baseURL!).href,
    async (route) => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: {
          ...response.headers(),
          "content-security-policy":
            "img-src 'self' data: https://avatars.githubusercontent.com",
        },
      });
    },
  );
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toBeVisible();
  const voice = page.locator(".workspace-list-entry").filter({
    has: page.locator(".workspace-list-item", { hasText: "Voice" }),
  });
  const customize = async () => {
    await voice
      .getByRole("button", { name: "Actions for Voice", exact: true })
      .click();
    await page
      .getByRole("menuitem", { name: "Customize workspace…", exact: true })
      .click();
  };
  await customize();
  const dialog = page.getByRole("dialog", {
    name: "Customize workspace",
    exact: true,
  });
  await dialog
    .getByRole("textbox", { name: "Custom emoji or text" })
    .fill("🚀");
  await dialog.getByLabel("Custom color", { exact: true }).fill("#6ba8ff");
  await page.screenshot({
    path: testInfo.outputPath("workspace-appearance-dark.png"),
  });
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect(voice.locator(".workspace-avatar")).toHaveText("🚀");
  await expect(voice.locator(".workspace-avatar")).toHaveCSS(
    "background-color",
    "rgb(107, 168, 255)",
  );
  await expect(
    page.locator(".workspace-list-item[aria-current=true]"),
  ).toContainText("Bench");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session") ?? "null")
            ?.projects?.[0]?.workspaces?.[0]?.appearance,
      ),
    )
    .toEqual({ icon: "🚀", color: "#6ba8ff" });
  await page.reload();
  await expect(voice.locator(".workspace-avatar")).toHaveText("🚀");
  await customize();
  await dialog.getByLabel("Workspace image", { exact: true }).setInputFiles({
    name: "avatar.png",
    mimeType: "image/png",
    buffer: Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=",
      "base64",
    ),
  });
  await expect(dialog.locator(".workspace-avatar img")).toBeVisible();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session")!).projects[0]
            .workspaces[0].appearance.image,
      ),
    )
    .toMatch(/^data:image\/png;base64,/);
  await page.reload();
  await expect(voice.locator(".workspace-avatar img")).toBeVisible();
  await customize();
  await dialog.getByRole("button", { name: "Reset", exact: true }).click();
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(voice.locator(".workspace-avatar img")).toBeVisible();
  await customize();
  await page.evaluate(() => {
    const original = HTMLImageElement.prototype.decode;
    (window as any).__restoreImageDecode = () => {
      HTMLImageElement.prototype.decode = original;
    };
    HTMLImageElement.prototype.decode = function () {
      return original.call(this).then(
        () =>
          new Promise<void>((resolve) => {
            (window as any).__releaseImageDecode = resolve;
          }),
      );
    };
  });
  await dialog.getByLabel("Workspace image", { exact: true }).setInputFiles({
    name: "replacement.png",
    mimeType: "image/png",
    buffer: Buffer.from(
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=",
      "base64",
    ),
  });
  await expect
    .poll(() =>
      page.evaluate(() => typeof (window as any).__releaseImageDecode),
    )
    .toBe("function");
  await dialog
    .getByRole("button", { name: "Remove image", exact: true })
    .click();
  await page.evaluate(() => {
    (window as any).__releaseImageDecode();
    (window as any).__restoreImageDecode();
  });
  await expect(dialog.locator(".workspace-avatar img")).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session")!).projects[0]
            .workspaces[0].appearance.image,
      ),
    )
    .toBeUndefined();
  await page.setViewportSize({ width: 800, height: 420 });
  await page.emulateMedia({ colorScheme: "light" });
  await customize();
  await expect(
    dialog.getByRole("button", { name: "Save", exact: true }),
  ).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("workspace-appearance-light-minimum.png"),
  });
  await dialog.getByRole("button", { name: "Reset", exact: true }).click();
  await dialog.getByRole("button", { name: "Save", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-session")!).projects[0]
            .workspaces[0].appearance,
      ),
    )
    .toBeUndefined();
});

test("workspace summaries track real CLI processes in split tabs without starting inactive terminals", async ({
  page,
}, testInfo) => {
  let session = addWorkspace(newSession(), "/project", "local:bash", "Build");
  const tab = active(session)!.tab;
  if (tab.type !== "terminal") throw new Error("Expected terminal");
  tab.layout = splitPane(tab.layout, tab.activePaneId, "horizontal", {
    type: "terminal",
    id: "second-pane",
    cwd: "/project",
  });
  session = addWorkspace(session, "/other", "local:bash", "Review");
  session.sidebar = "workspaces";
  session.sidebarWidth = 300;
  session.activeProjectId = session.projects[0].id;
  await mockDesktop(page, true, session);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() => {
    const native = (window as any).__nativeTest;
    const original = (window as any).__TAURI_INTERNALS__.invoke;
    (window as any).__TAURI_INTERNALS__.invoke = async (
      command: string,
      args: any,
    ) => {
      if (command === "git_repositories")
        return {
          repositories:
            args.root === "/project"
              ? [
                  { root: "/project", branch: "main", changes: [] },
                  { root: "/project/nested", branch: "main", changes: [] },
                ]
              : [],
          errors: [],
          limited: false,
        };
      return original(command, args);
    };
    native.terminalContexts = {};
  });
  await page.goto("/");
  await expect(page.locator(".xterm-screen")).toHaveCount(2);
  const build = page.locator(".workspace-list-entry").filter({
    has: page.locator(".workspace-list-item", { hasText: "Build" }),
  });
  const review = page.locator(".workspace-list-entry").filter({
    has: page.locator(".workspace-list-item", { hasText: "Review" }),
  });
  await expect(build.locator(".workspace-repository-count")).toHaveText(
    "2 repos",
  );
  await expect(review.locator(".workspace-repository-count")).toHaveText(
    "0 repos",
  );
  await page.evaluate(() => {
    const native = (window as any).__nativeTest;
    const starts = native.calls.filter(
      (call: any) => call.command === "start_terminal",
    );
    native.terminalContexts = Object.fromEntries(
      starts.map((call: any, index: number) => [
        call.args.request.id,
        {
          cwd: "/project",
          foregroundProgram: index ? "claude" : "codex",
          titleCli: { cli: index ? "claude" : "codex", pid: 200 + index },
        },
      ]),
    );
  });
  await expect(build.locator(".workspace-agent-count")).toHaveText("2 agents");
  await expect(build.locator(".workspace-agent-chip")).toHaveText(["", ""]);
  await expect(
    build.getByRole("img", { name: "Codex · 1 agent", exact: true }),
  ).toBeVisible();
  await expect(
    build.getByRole("img", { name: "Claude Code · 1 agent", exact: true }),
  ).toBeVisible();
  await expect(review.locator(".workspace-agent-count")).toHaveText("0 agents");
  await build
    .getByRole("button", { name: "Expand tabs in Build", exact: true })
    .click();
  await expect(build.locator(".workspace-tab-agents")).toHaveText("2");
  await page.screenshot({
    path: testInfo.outputPath("workspace-agents-dark.png"),
  });
  await review.locator(".workspace-list-item").click();
  await expect(build.locator(".workspace-agent-count")).toHaveText("2 agents");
  expect(
    await page.evaluate(
      () =>
        (window as any).__nativeTest.calls.filter(
          (call: any) => call.command === "start_terminal",
        ).length,
    ),
  ).toBe(3);
  await page.evaluate(() => {
    (window as any).__nativeTest.terminalContexts = {};
  });
  await expect(build.locator(".workspace-agent-count")).toHaveText("0 agents");
  await expect(build.locator(".workspace-agent-chip")).toHaveCount(0);
});
