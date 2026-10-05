import { expect, test, type Page, type Locator } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { cliNames } from "../../src/cli-agents";

async function chooseCli(page: Page, cli: string) {
  const change = page.getByRole("button", { name: "Change CLI", exact: true });
  const wizard = page.getByRole("dialog", { name: "New router", exact: true });
  if (await change.isVisible()) await change.click();
  else if (!(await wizard.isVisible()))
    await page
      .getByRole("button", { name: "New router", exact: true })
      .first()
      .click();
  await page
    .getByRole("searchbox", { name: "Search agents", exact: true })
    .fill(cli);
  const names = cliNames as Record<string, string>;
  await page
    .locator(".router-cli-option")
    .filter({ hasText: names[cli] || cli })
    .first()
    .click();
}

async function routerWizardFixture(page: Page, empty = false) {
  await mockDesktop(page, false, null);
  await page.addInitScript(
    ({ names, empty }) => {
      const desktop = window as any;
      const original = desktop.__TAURI_INTERNALS__.invoke;
      const snapshot: any = {
        revision: 1,
        profiles: empty
          ? []
          : [
              { id: "personal", cli: "codex", label: "Personal" },
              { id: "work", cli: "codex", label: "Work" },
              { id: "team", cli: "claude", label: "Team" },
            ].map((profile) => ({
              ...profile,
              enabled: true,
              revision: 1,
              authState: "ready",
              storageMode: "cli_managed",
            })),
        routers: empty
          ? []
          : [
              {
                id: "daily",
                cli: "codex",
                label: "Daily",
                enabled: true,
                orderedProfileIds: ["personal", "work"],
                balanceRemainingQuota: false,
                revision: 1,
              },
              {
                id: "team-router",
                cli: "claude",
                label: "Team router",
                enabled: false,
                orderedProfileIds: ["team"],
                balanceRemainingQuota: false,
                revision: 1,
              },
            ],
        quota: [],
        runs: [],
        capabilities: Object.entries(names).map(([cli, name]) => ({
          cli,
          name,
          canCreateProfile: true,
          canVerifyLogin: cli === "codex",
          profileTerminal: true,
          quotaRead: cli === "codex",
          balance: cli === "codex",
        })),
      };
      desktop.routerWizardCalls = [];
      desktop.routerWizardSnapshot = () => structuredClone(snapshot);
      desktop.changeWizardData = async (kind: string) => {
        if (kind === "router") snapshot.routers[0].revision++;
        if (kind === "missing")
          snapshot.profiles = snapshot.profiles.filter(
            (p: any) => p.id !== "work",
          );
        snapshot.revision++;
        await desktop.__nativeTest.emitEvent("cli-router-changed", {
          revision: snapshot.revision,
        });
      };
      desktop.__TAURI_INTERNALS__.invoke = async (
        command: string,
        args: any,
      ) => {
        if (command === "cli_router_snapshot") return structuredClone(snapshot);
        if (command === "cli_profile_native_report") return null;
        if (command === "cli_router_mutate") {
          const request = args.request;
          desktop.routerWizardCalls.push(structuredClone(request));
          if (request.expectedRevision !== snapshot.revision)
            throw new Error("Stale revision");
          const action = request.action;
          if (action.type === "create_profile")
            snapshot.profiles.push({
              id: `profile-${snapshot.profiles.length + 1}`,
              cli: action.cli,
              label: action.label,
              enabled: true,
              authState: "disconnected",
              storageMode: "cli_managed",
              revision: 1,
            });
          if (action.type === "create_router") {
            if (desktop.failCreate)
              throw new Error(
                "Verify two compatible accounts before enabling this router.",
              );
            snapshot.routers.push({
              ...action,
              id: "created",
              enabled: action.enabled ?? false,
              balanceRemainingQuota: action.balanceRemainingQuota ?? false,
              revision: 1,
            });
          }
          if (action.type === "update_router") {
            const router = snapshot.routers.find(
              (r: any) => r.id === action.routerId,
            );
            Object.assign(router, action, { revision: router.revision + 1 });
          }
          if (action.type === "remove_router") {
            if (desktop.failRemove)
              throw new Error("Stop active runs before removing this router.");
            snapshot.routers = snapshot.routers.filter(
              (r: any) => r.id !== action.routerId,
            );
          }
          if (action.type === "remove_profile") {
            if (desktop.pendingCleanup) {
              const profile = snapshot.profiles.find(
                (p: any) => p.id === action.profileId,
              );
              profile.authState = "pending_remove";
              profile.enabled = false;
              profile.revision += 2;
              for (const router of snapshot.routers) {
                router.orderedProfileIds = router.orderedProfileIds.filter(
                  (id: string) => id !== action.profileId,
                );
                router.revision++;
                if (!router.orderedProfileIds.length) router.enabled = false;
              }
              snapshot.revision++;
              desktop.pendingCleanup = false;
              throw new Error("API key removal is pending. Retry removal.");
            }
            snapshot.profiles = snapshot.profiles.filter(
              (p: any) => p.id !== action.profileId,
            );
            for (const router of snapshot.routers) {
              router.orderedProfileIds = router.orderedProfileIds.filter(
                (id: string) => id !== action.profileId,
              );
              router.revision++;
              if (!router.orderedProfileIds.length) router.enabled = false;
            }
            if (desktop.changeOnRemove) {
              snapshot.routers[0].label = "Changed elsewhere";
              snapshot.routers[0].revision++;
            }
          }
          snapshot.revision++;
          return structuredClone(snapshot);
        }
        if (
          [
            "cli_run_start",
            "cli_run_send",
            "cli_profile_open_terminal",
            "cli_profile_verify",
          ].includes(command)
        ) {
          desktop.routerWizardCalls.push({ command });
          throw new Error("Router setup must not launch CLI work");
        }
        return original(command, args);
      };
    },
    { names: cliNames, empty },
  );
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
}

test("new router selects a CLI and saves all reviewed settings once", async ({
  page,
}) => {
  await routerWizardFixture(page);
  await expect(
    page.getByRole("button", { name: "Manage Daily", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Manage Team router", exact: true }),
  ).toBeVisible();
  await chooseCli(page, "codex");
  const wizard = page.getByRole("dialog", { name: "New router", exact: true });
  await wizard
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("Coding");
  await wizard
    .getByRole("button", { name: "Move Work up", exact: true })
    .click();
  await wizard
    .getByRole("switch", { name: "Enable router", exact: true })
    .check();
  await wizard.getByText("Advanced", { exact: true }).click();
  await wizard
    .getByRole("switch", {
      name: "Balance remaining quota for Coding",
      exact: true,
    })
    .check();
  expect(await page.evaluate(() => (window as any).routerWizardCalls)).toEqual(
    [],
  );
  await wizard
    .getByRole("button", { name: "Create router", exact: true })
    .click();
  await expect(wizard).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Manage Coding", exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => (window as any).routerWizardCalls)).toEqual([
    expect.objectContaining({
      expectedRevision: 1,
      action: {
        type: "create_router",
        cli: "codex",
        label: "Coding",
        orderedProfileIds: ["work", "personal"],
        enabled: true,
        balanceRemainingQuota: true,
      },
    }),
  ]);
  await page.screenshot({ path: "test-results/router-created-dashboard.png" });
});

test("changing CLI resets account selections and cancellation discards the router draft", async ({
  page,
}) => {
  await routerWizardFixture(page);
  await chooseCli(page, "codex");
  const wizard = page.getByRole("dialog", { name: "New router", exact: true });
  await wizard
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("Draft");
  await wizard.getByText("Advanced", { exact: true }).click();
  await wizard
    .getByRole("switch", {
      name: "Balance remaining quota for Draft",
      exact: true,
    })
    .check();
  await chooseCli(page, "claude");
  await expect(
    wizard.getByRole("button", { name: "Manage Team", exact: true }),
  ).toBeVisible();
  await expect(
    wizard.getByRole("button", { name: "Manage Work", exact: true }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(wizard).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).routerWizardCalls)).toEqual(
    [],
  );
  await chooseCli(page, "codex");
  await expect(
    wizard.getByRole("textbox", { name: "Router name", exact: true }),
  ).not.toHaveValue("Draft");
  await wizard
    .getByRole("button", { name: "Create router", exact: true })
    .click();
  await expect(wizard).toHaveCount(0);
  const action = await page.evaluate(
    () => (window as any).routerWizardCalls[0].action,
  );
  expect(action.enabled).toBe(false);
  expect(action.balanceRemainingQuota).toBe(false);
  expect(action.orderedProfileIds).toEqual(["personal", "work"]);
});

test("nested account setup keeps the router draft and saves accounts independently", async ({
  page,
}) => {
  await routerWizardFixture(page, true);
  await chooseCli(page, "codex");
  const wizard = page.getByRole("dialog", { name: "New router", exact: true });
  await wizard
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("Keep my draft");
  await expect(
    wizard.getByRole("button", { name: "Create router", exact: true }),
  ).toBeDisabled();
  await wizard
    .getByRole("button", { name: "Add account", exact: true })
    .click();
  const add = page.getByRole("dialog", { name: "Add account", exact: true });
  await add
    .getByRole("textbox", { name: "Account name", exact: true })
    .fill("Personal");
  await add.getByRole("button", { name: "Add account", exact: true }).click();
  const account = page.getByRole("dialog", { name: "Personal", exact: true });
  await expect(account).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(account).toHaveCount(0);
  await expect(wizard).toBeVisible();
  await expect(
    wizard.getByRole("textbox", { name: "Router name", exact: true }),
  ).toHaveValue("Keep my draft");
  await expect(
    wizard.getByRole("button", { name: "Create router", exact: true }),
  ).toBeEnabled();
  await wizard.getByRole("button", { name: "Cancel", exact: true }).click();
  const saved = await page.evaluate(() =>
    (window as any).routerWizardSnapshot(),
  );
  expect(saved.routers).toEqual([]);
  expect(saved.profiles.map((p: any) => p.label)).toEqual(["Personal"]);
  await chooseCli(page, "codex");
  await expect(
    wizard.getByRole("button", { name: "Manage Personal", exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => (window as any).routerWizardCalls)).toEqual([
    expect.objectContaining({
      expectedRevision: 1,
      action: { type: "create_profile", cli: "codex", label: "Personal" },
    }),
  ]);
});

test("failed router creation retains reviewed fields for retry", async ({
  page,
}) => {
  await routerWizardFixture(page);
  await chooseCli(page, "codex");
  const wizard = page.getByRole("dialog", { name: "New router", exact: true });
  await wizard
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("Retained draft");
  await wizard
    .getByRole("switch", { name: "Enable router", exact: true })
    .check();
  await page.evaluate(() => {
    (window as any).failCreate = true;
  });
  await wizard
    .getByRole("button", { name: "Create router", exact: true })
    .click();
  await expect(
    wizard.getByText(
      "Verify two compatible accounts before enabling this router.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    wizard.getByRole("textbox", { name: "Router name", exact: true }),
  ).toHaveValue("Retained draft");
  expect(
    (await page.evaluate(() => (window as any).routerWizardSnapshot())).routers,
  ).toHaveLength(2);
  await page.evaluate(() => {
    (window as any).failCreate = false;
  });
  await wizard
    .getByRole("switch", { name: "Enable router", exact: true })
    .uncheck();
  await wizard
    .getByRole("button", { name: "Create router", exact: true })
    .click();
  await expect(wizard).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Manage Retained draft", exact: true }),
  ).toBeVisible();
});

test("router editor blocks stale or missing-account saves", async ({
  page,
}) => {
  await routerWizardFixture(page);
  await page.getByRole("button", { name: "Manage Daily", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "Edit router", exact: true });
  await editor
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("My edit");
  await page.evaluate(() => (window as any).changeWizardData("router"));
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  await editor
    .locator("form")
    .evaluate((form) =>
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      ),
    );
  expect(await page.evaluate(() => (window as any).routerWizardCalls)).toEqual(
    [],
  );
  await editor.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("button", { name: "Manage Daily", exact: true }).click();
  await page.evaluate(() => (window as any).changeWizardData("missing"));
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  await editor
    .locator("form")
    .evaluate((form) =>
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      ),
    );
  expect(await page.evaluate(() => (window as any).routerWizardCalls)).toEqual(
    [],
  );
});

test("router deletion keeps accounts and failed removal preserves the editor draft", async ({
  page,
}) => {
  await routerWizardFixture(page);
  await page.getByRole("button", { name: "Manage Daily", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "Edit router", exact: true });
  await editor
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("Keep my changes");
  await editor
    .getByRole("button", { name: "Remove router Keep my changes", exact: true })
    .click();
  const removal = page.getByRole("alertdialog", {
    name: "Remove Keep my changes?",
    exact: true,
  });
  await expect(
    removal.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(removal).toHaveCount(0);
  await expect(
    editor.getByRole("textbox", { name: "Router name", exact: true }),
  ).toHaveValue("Keep my changes");
  expect(await page.evaluate(() => (window as any).routerWizardCalls)).toEqual(
    [],
  );
  await editor
    .getByRole("button", { name: "Remove router Keep my changes", exact: true })
    .click();
  await page.evaluate(() => {
    (window as any).failRemove = true;
  });
  await removal
    .getByRole("button", { name: "Remove router", exact: true })
    .click();
  await expect(
    removal.getByText("Stop active runs before removing this router.", {
      exact: true,
    }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(
    editor.getByRole("textbox", { name: "Router name", exact: true }),
  ).toHaveValue("Keep my changes");
  await page.evaluate(() => {
    (window as any).failRemove = false;
  });
  await editor
    .getByRole("button", { name: "Remove router Keep my changes", exact: true })
    .click();
  await removal
    .getByRole("button", { name: "Remove router", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  const saved = await page.evaluate(() =>
    (window as any).routerWizardSnapshot(),
  );
  expect(saved.routers.map((r: any) => r.id)).toEqual(["team-router"]);
  expect(saved.profiles.map((p: any) => p.id)).toEqual([
    "personal",
    "work",
    "team",
  ]);
});

for (const editing of [false, true]) {
  test(`successful nested account removal preserves ${editing ? "existing" : "new"} router settings`, async ({
    page,
  }) => {
    await routerWizardFixture(page);
    if (editing)
      await page
        .getByRole("button", { name: "Manage Daily", exact: true })
        .click();
    else await chooseCli(page, "codex");
    const editor = page.getByRole("dialog", {
      name: editing ? "Edit router" : "New router",
      exact: true,
    });
    await editor
      .getByRole("textbox", { name: "Router name", exact: true })
      .fill("Keep my setup");
    await editor
      .getByRole("switch", { name: "Enable router", exact: true })
      .uncheck();
    await editor.getByText("Advanced", { exact: true }).click();
    await editor
      .getByRole("switch", {
        name: "Balance remaining quota for Keep my setup",
        exact: true,
      })
      .check();
    await editor
      .getByRole("button", { name: "Manage Work", exact: true })
      .click();
    const account = page.getByRole("dialog", { name: "Work", exact: true });
    await account
      .getByRole("button", { name: "Remove account Work", exact: true })
      .click();
    await page
      .getByRole("alertdialog", { name: "Remove Work?", exact: true })
      .getByRole("button", { name: "Remove account", exact: true })
      .click();
    await expect(account).toHaveCount(0);
    await expect(
      editor.getByRole("textbox", { name: "Router name", exact: true }),
    ).toHaveValue("Keep my setup");
    await expect(
      editor.getByRole("switch", {
        name: "Balance remaining quota for Keep my setup",
        exact: true,
      }),
    ).toBeChecked();
    const save = editor.getByRole("button", {
      name: editing ? "Save changes" : "Create router",
      exact: true,
    });
    await expect(save).toBeEnabled();
    await save.click();
    await expect(editor).toHaveCount(0);
    const calls = await page.evaluate(() => (window as any).routerWizardCalls);
    expect(calls).toHaveLength(2);
    expect(calls[1]).toEqual(
      expect.objectContaining({
        expectedRevision: 2,
        action: expect.objectContaining({
          type: editing ? "update_router" : "create_router",
          label: "Keep my setup",
          orderedProfileIds: ["personal"],
          enabled: false,
          balanceRemainingQuota: true,
        }),
      }),
    );
  });
}

test("nested account removal does not accept unrelated router changes", async ({
  page,
}) => {
  await routerWizardFixture(page);
  await page.getByRole("button", { name: "Manage Daily", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "Edit router", exact: true });
  await editor
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("My reviewed draft");
  await editor
    .getByRole("button", { name: "Manage Work", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Work", exact: true })
    .getByRole("button", { name: "Remove account Work", exact: true })
    .click();
  await page.evaluate(() => {
    (window as any).changeOnRemove = true;
  });
  await page
    .getByRole("alertdialog", { name: "Remove Work?", exact: true })
    .getByRole("button", { name: "Remove account", exact: true })
    .click();
  await expect(
    editor.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeDisabled();
  await expect(
    editor.getByRole("textbox", { name: "Router name", exact: true }),
  ).toHaveValue("My reviewed draft");
  await editor
    .locator("form")
    .evaluate((form) =>
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      ),
    );
  expect(
    await page.evaluate(() => (window as any).routerWizardCalls),
  ).toHaveLength(1);
});

test("pending credential cleanup can be retried without discarding the router draft", async ({
  page,
}) => {
  await routerWizardFixture(page);
  await page.getByRole("button", { name: "Manage Daily", exact: true }).click();
  const editor = page.getByRole("dialog", { name: "Edit router", exact: true });
  await editor
    .getByRole("textbox", { name: "Router name", exact: true })
    .fill("Keep my draft");
  await editor
    .getByRole("switch", { name: "Enable router", exact: true })
    .uncheck();
  await editor.getByText("Advanced", { exact: true }).click();
  await editor
    .getByRole("switch", {
      name: "Balance remaining quota for Keep my draft",
      exact: true,
    })
    .check();
  await editor
    .getByRole("button", { name: "Manage Work", exact: true })
    .click();
  const account = page.getByRole("dialog", { name: "Work", exact: true });
  await account
    .getByRole("button", { name: "Remove account Work", exact: true })
    .click();
  const removal = page.getByRole("alertdialog", {
    name: "Remove Work?",
    exact: true,
  });
  await page.evaluate(() => {
    (window as any).pendingCleanup = true;
  });
  await removal
    .getByRole("button", { name: "Remove account", exact: true })
    .click();
  await expect(
    removal.getByText("API key removal is pending. Retry removal.", {
      exact: true,
    }),
  ).toBeVisible();
  await removal.getByRole("button", { name: "Cancel", exact: true }).click();
  await account
    .getByRole("button", { name: "Remove account Work", exact: true })
    .click();
  await removal
    .getByRole("button", { name: "Remove account", exact: true })
    .click();
  await expect(account).toHaveCount(0);
  await expect(
    editor.getByRole("textbox", { name: "Router name", exact: true }),
  ).toHaveValue("Keep my draft");
  await expect(
    editor.getByRole("switch", {
      name: "Balance remaining quota for Keep my draft",
      exact: true,
    }),
  ).toBeChecked();
  const save = editor.getByRole("button", {
    name: "Save changes",
    exact: true,
  });
  await expect(save).toBeEnabled();
  await save.click();
  await expect(editor).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).routerWizardCalls);
  expect(calls.map((call: any) => call.expectedRevision)).toEqual([1, 2, 3]);
  expect(calls[2].action).toEqual(
    expect.objectContaining({
      type: "update_router",
      routerId: "daily",
      label: "Keep my draft",
      orderedProfileIds: ["personal"],
      enabled: false,
      balanceRemainingQuota: true,
    }),
  );
});
async function chooseRoute(scope: Locator, id: string) {
  await expect(scope).toBeVisible();
  if (!(await scope.locator(".router-launch-heading").count()))
    await scope
      .getByRole("button", { name: "New run", exact: true })
      .first()
      .click();
  await expect(scope.locator(".router-launch-heading")).toBeVisible();
  while (
    !(await scope
      .getByRole("radiogroup", { name: "Router", exact: true })
      .isVisible())
  )
    await scope.getByRole("button", { name: "Back", exact: true }).click();
  await scope
    .locator(`label:has(input[name="launch-router"][value="${id}"])`)
    .click();
  await scope.getByRole("button", { name: "Next", exact: true }).click();
  const modes = scope.getByRole("radiogroup", {
    name: "Execution mode",
    exact: true,
  });
  if (
    (await modes.isVisible()) &&
    (await modes.locator("input:checked").count())
  )
    await scope.getByRole("button", { name: "Next", exact: true }).click();
}
async function chooseMode(scope: Locator, mode: string) {
  const modes = scope.getByRole("radiogroup", {
    name: "Execution mode",
    exact: true,
  });
  if (!(await modes.isVisible()))
    await scope.getByRole("button", { name: "Back", exact: true }).click();
  await modes.locator(`label:has(input[value="${mode}"])`).click();
  await scope.getByRole("button", { name: "Next", exact: true }).click();
}

test("saved history requires explicit account consent and rejects stale review", async ({
  page,
}) => {
  const { newProject, newSession, newCliAgentTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const tab = newCliAgentTab("grant-run", "Private task");
  project.workspaces[0].tabs = [tab];
  project.workspaces[0].activeTabId = tab.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot = {
      revision: 1,
      profiles: ["first", "later"].map((id) => ({
        id,
        cli: "gemini",
        label: id === "first" ? "Original account" : "Later account",
        enabled: true,
        revision: 1,
        authState: "ready",
        storageMode: "api_key",
      })),
      routers: [
        {
          id: "pool",
          cli: "gemini",
          label: "Pool",
          enabled: true,
          orderedProfileIds: ["first", "later"],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      quota: [],
      capabilities: [{ cli: "gemini", name: "Gemini CLI", managedTurns: true }],
      runs: [
        {
          id: "grant-run",
          routerId: "pool",
          cwd: "/project",
          shellProfileId: null,
          title: "Private task",
          state: "recovery_required",
          model: "gemini-model",
          pinnedProfileId: "first",
          allowedProfileIds: ["first"],
          activeProfileId: null,
          generation: 1,
          revision: 1,
          inputs: [{ id: "input", text: "Private saved question" }],
          attempts: [],
          turns: [],
          output: "Uncertain answer",
          statusMessage: "",
          attemptedProfileIds: [],
        },
      ],
    };
    desktop.grantCalls = [];
    desktop.changeReviewedAccount = async () => {
      snapshot.profiles[1].revision++;
      snapshot.revision++;
      await desktop.__nativeTest.emitEvent("cli-router-changed", {
        revision: snapshot.revision,
      });
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_run_update_data_grant") {
        const request = args.request;
        desktop.grantCalls.push({ command, ...request });
        if (
          request.expectedRevision !== snapshot.runs[0].revision ||
          request.profiles.some(
            (p: any) =>
              snapshot.profiles.find((item) => item.id === p.profileId)
                ?.revision !== p.expectedRevision,
          )
        )
          throw new Error("Stale reviewed identity");
        snapshot.runs[0].allowedProfileIds = request.allowedProfileIds;
        snapshot.runs[0].revision++;
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      if (["cli_run_send", "cli_run_start"].includes(command)) {
        desktop.grantCalls.push({ command, ...args });
        throw new Error("Account consent must not start a task");
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  const pane = page.getByRole("region", { name: "Private task", exact: true });
  await expect(
    pane.getByLabel("Account for next attempt").locator("option"),
  ).toHaveText(["Router order", "Original account"]);
  await pane
    .getByRole("button", { name: "Review account access", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Review access to saved history",
    exact: true,
  });
  const later = review.getByRole("checkbox", {
    name: "Later account · new access",
    exact: true,
  });
  await expect(later).not.toBeChecked();
  await expect(
    review.getByText(/including uncertain partial responses/),
  ).toBeVisible();
  await later.check();
  await page.evaluate(() => (window as any).changeReviewedAccount());
  await expect(
    review.getByRole("button", { name: "Save account access", exact: true }),
  ).toBeDisabled();
  await expect(
    review.getByText(/The task or an account changed/),
  ).toBeVisible();
  await review
    .locator("form")
    .evaluate((form) =>
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      ),
    );
  expect(await page.evaluate(() => (window as any).grantCalls)).toEqual([]);
  await review.getByRole("button", { name: "Cancel", exact: true }).click();
  await pane
    .getByRole("button", { name: "Review account access", exact: true })
    .click();
  await review
    .getByRole("checkbox", { name: "Later account · new access", exact: true })
    .check();
  await review
    .getByRole("button", { name: "Save account access", exact: true })
    .click();
  await expect(review).toHaveCount(0);
  await expect(
    pane.getByLabel("Account for next attempt").locator("option"),
  ).toHaveText(["Router order", "Original account", "Later account"]);
  expect(await page.evaluate(() => (window as any).grantCalls)).toEqual([
    expect.objectContaining({
      command: "cli_run_update_data_grant",
      runId: "grant-run",
      expectedRevision: 1,
      allowedProfileIds: ["first", "later"],
      profiles: [
        { profileId: "first", expectedRevision: 1 },
        { profileId: "later", expectedRevision: 2 },
      ],
    }),
  ]);
  await expect(
    pane.getByRole("button", { name: "Continue saved task", exact: true }),
  ).toBeVisible();
  const runDialog = await openNativeModelDialog(page);
  await runDialog.getByRole("button", { name: /^History/ }).click();
  if (await runDialog.getByText("All runs", { exact: true }).isVisible())
    await runDialog.getByText("All runs", { exact: true }).click();
  await runDialog
    .getByRole("combobox", { name: "Run history", exact: true })
    .selectOption("grant-run");
  await runDialog
    .getByRole("button", { name: "Review account access", exact: true })
    .click();
  await expect(review).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(review).toHaveCount(0);
  await expect(runDialog).toBeVisible();
  await expect(
    runDialog.getByRole("combobox", { name: "Run history", exact: true }),
  ).toHaveValue("grant-run");
});

test("router excludes removed CLIs and saves account order with revision fencing", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript((names) => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot = {
      revision: 1,
      capabilities: Object.entries(names).map(([cli, name]) => ({
        cli,
        name,
        canCreateProfile: ["codex", "claude", "gemini", "pi"].includes(cli),
        canVerifyLogin: cli === "codex" || cli === "claude",
        canStart: ["codex", "claude", "gemini", "pi"].includes(cli),
        profileTerminal: ["codex", "claude", "gemini", "pi"].includes(cli),
        managedTurns: ["claude", "gemini", "pi"].includes(cli),
        sameAccountResume: false,
        crossAccountResume: false,
        quotaRead: cli === "codex",
        balance: cli === "codex",
        readOnly: false,
        reason:
          cli === "codex"
            ? "Separate account terminals and quota are available. Managed runs are unavailable."
            : "Separate accounts are not supported for this CLI yet.",
      })),
      profiles: [
        ...["first", "second"].map((id) => ({
          id,
          cli: "codex",
          label: id === "first" ? "Personal" : "Work",
          enabled: true,
          revision: 1,
          authState: "ready",
          storageMode: "cli_managed",
        })),
      ],
      routers: [
        {
          id: "pool",
          cli: "codex",
          label: "Daily router",
          enabled: true,
          orderedProfileIds: ["first", "second"],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      quota: [],
      runs: [],
    };
    desktop.routerCalls = [];
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_router_mutate") {
        desktop.routerCalls.push(args.request);
        if (args.request.expectedRevision !== snapshot.revision)
          throw new Error("Stale revision");
        if (args.request.action.type === "remove_profile") {
          snapshot.profiles[0].authState = "pending_remove";
          snapshot.profiles[0].enabled = false;
          snapshot.revision++;
          throw new Error("API key removal is pending. Retry removal.");
        }
        Object.assign(snapshot.routers[0], args.request.action);
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      return original(command, args);
    };
  }, cliNames);
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
  await expect(page.locator(".router-cli-option")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Manage Daily router", exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/router-ux-dashboard.png" });
  await page.getByRole("button", { name: "New router", exact: true }).click();
  await expect(
    page.getByRole("searchbox", { name: "Search agents", exact: true }),
  ).toBeFocused();
  await expect(page.locator(".router-cli-option")).toHaveText([
    "Codex",
    "Antigravity CLI",
    "Cursor CLI",
    "Claude Code",
    "Gemini CLI",
    "GitHub Copilot CLI",
    "OpenCode",
    "OpenClaw",
    "Hermes Agent",
    "Pi Coding Agent",
    "Kilo Code CLI",
    "Qwen Code",
    "Kiro CLI",
    "Mistral Vibe",
    "Kimi Code CLI",
    "Grok Build",
  ]);
  await page.screenshot({ path: "test-results/router-cli-catalog.png" });
  await page.setViewportSize({ width: 800, height: 600 });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await page.screenshot({
    path: "test-results/router-cli-picker-compact-dark.png",
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: "light" });
  await page.getByRole("searchbox", { name: "Search agents" }).fill("Aider");
  await expect(page.locator(".router-cli-option")).toHaveCount(0);
  await expect(
    page.getByText("No matching CLI.", { exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => (window as any).routerCalls)).toEqual([]);
  await chooseCli(page, "cursor");
  await expect(
    page.getByText(
      "Account routing is unavailable for this agent. See support details.",
      {
        exact: true,
      },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add account", exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("dialog", { name: "New router", exact: true })
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Manage Daily router", exact: true })
    .click();
  const route = page.getByRole("dialog", { name: "Edit router", exact: true });
  await route
    .getByRole("button", { name: "Move Work up", exact: true })
    .click();
  await expect(
    route
      .getByRole("list", { name: "Account order for Daily router" })
      .getByRole("listitem")
      .first(),
  ).toContainText("Work");
  expect(await page.evaluate(() => (window as any).routerCalls)).toEqual([]);
  await page.screenshot({ path: "test-results/router-ux-route-order.png" });
  await route
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).routerCalls[0]?.expectedRevision),
    )
    .toBe(1);
  await page
    .getByRole("button", { name: "Manage Daily router", exact: true })
    .click();
  await route.getByText("Advanced", { exact: true }).click();
  await route
    .getByRole("switch", {
      name: "Balance remaining quota for Daily router",
      exact: true,
    })
    .check();
  await route
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(page.getByText("Account order", { exact: true })).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).routerCalls[1]?.expectedRevision),
    )
    .toBe(2);
  await page
    .getByRole("button", { name: "Manage Daily router", exact: true })
    .click();
  await page
    .getByRole("button", { name: "CLI support details", exact: true })
    .click();
  const support = page.getByRole("dialog", { name: "Codex", exact: true });
  await expect(
    support.getByText(
      "Separate account terminals and quota are available. Managed runs are unavailable.",
      { exact: true },
    ),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await page
    .getByRole("button", { name: "Manage Personal", exact: true })
    .click();
  const account = page.getByRole("dialog", { name: "Personal", exact: true });
  await account
    .getByRole("button", { name: "Remove account Personal", exact: true })
    .click();
  const removal = page.getByRole("alertdialog", {
    name: "Remove Personal?",
    exact: true,
  });
  await expect(
    removal.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await removal
    .getByRole("button", { name: "Remove account", exact: true })
    .click();
  await expect(
    removal.getByText("API key removal is pending. Retry removal.", {
      exact: true,
    }),
  ).toBeVisible();
  await removal.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    account.getByText("Removal pending", { exact: true }),
  ).toBeVisible();
  await expect(
    account.getByRole("button", { name: "Sign in with CLI", exact: true }),
  ).toBeDisabled();
  await expect(
    account.getByRole("button", { name: "Verify", exact: true }),
  ).toBeDisabled();
  await expect(
    account.getByRole("button", {
      name: "Remove account Personal",
      exact: true,
    }),
  ).toBeEnabled();
  const enabled = page.getByRole("switch", {
    name: "Enable Personal",
    exact: true,
  });
  await expect(enabled).toBeDisabled();
  await expect(enabled).not.toBeChecked();
  await account.getByRole("button", { name: "Done", exact: true }).click();
  await route.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.screenshot({ path: "test-results/router-settings.png" });
});

test("routed run requires a model and explicit recovery handoff", async ({
  page,
}) => {
  await mockDesktop(page, false, undefined, undefined, undefined, undefined, {
    installed: [{ cli: "gemini", name: "Gemini CLI", command: "gemini" }],
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot: any = {
      revision: 1,
      profiles: [
        {
          id: "first",
          cli: "gemini",
          label: "Personal",
          enabled: true,
          revision: 1,
          authState: "ready",
          storageMode: "api_key",
        },
      ],
      routers: [
        {
          id: "pool",
          cli: "gemini",
          label: "Daily router",
          enabled: true,
          orderedProfileIds: ["first"],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      capabilities: [
        {
          cli: "gemini",
          name: "Gemini CLI",
          managedTurns: true,
          reason: "API-key text-only turns. Fresh conversation for every turn.",
        },
      ],
      quota: [],
      runs: [],
    };
    desktop.routerRunCalls = [];
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_run_start") {
        desktop.routerRunCalls.push({ command, ...args.request });
        snapshot.runs.push({
          id: args.request.requestId,
          routerId: "pool",
          cwd: "/project",
          title: args.request.title,
          state: "idle",
          model: args.request.model,
          pinnedProfileId: null,
          allowedProfileIds: ["first"],
          activeProfileId: null,
          generation: 1,
          revision: 1,
          inputs: [],
          attempts: [],
          output: "",
          statusMessage: "",
          attemptedProfileIds: [],
        });
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      if (command === "cli_run_remove") {
        desktop.routerRunCalls.push({ command, ...args.request });
        if (args.request.expectedRevision !== snapshot.runs[0].revision)
          throw new Error("Stale run revision");
        snapshot.runs = [];
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      if (command === "cli_run_send") {
        desktop.routerRunCalls.push({ command, ...args.request });
        if (!args.request.handoff)
          snapshot.runs[0].inputs.push({
            id: "input",
            text: args.request.text,
          });
        snapshot.runs[0].state = args.request.handoff
          ? "completed"
          : "recovery_required";
        snapshot.runs[0].output = "Partial CLI output";
        snapshot.runs[0].revision++;
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Agents", exact: true })
    .getByRole("button", { name: "Open router", exact: true })
    .click();
  const dialog = page.locator("dialog.router-launch-dialog");
  await chooseRoute(dialog, "pool");
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toBeDisabled();
  await expect(dialog.getByText("Chat", { exact: true })).toBeVisible();
  await dialog.getByLabel("Model", { exact: true }).fill("gemini-2.5-pro");
  await dialog.getByRole("button", { name: "Start run", exact: true }).click();
  const pane = page.getByRole("region", { name: "CLI run", exact: true });
  await pane.getByLabel("Message", { exact: true }).fill("Review the project");
  await pane.getByRole("button", { name: "Send", exact: true }).click();
  await expect(
    pane.getByRole("button", {
      name: "Continue saved task",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    pane.getByRole("button", { name: "Send", exact: true }),
  ).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).routerRunCalls.length)).toBe(
    2,
  );
  await page.screenshot({ path: "test-results/router-run-recovery.png" });
  await pane
    .getByRole("button", { name: "Continue saved task", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).routerRunCalls[2]?.handoff))
    .toBe(true);
  await expect(
    pane.getByRole("button", {
      name: "Continue saved task",
      exact: true,
    }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Agents", exact: true })
    .getByRole("button", { name: "Open router", exact: true })
    .click();
  await dialog.getByRole("button", { name: /^History/ }).click();
  if (await dialog.getByText("All runs", { exact: true }).isVisible())
    await dialog.getByText("All runs", { exact: true }).click();
  await dialog
    .getByRole("combobox", { name: "Run history", exact: true })
    .selectOption(
      await page.evaluate(() => (window as any).routerRunCalls[0].requestId),
    );
  await dialog
    .getByRole("button", { name: "Remove from history", exact: true })
    .click();
  const removal = page.getByRole("dialog", {
    name: "Remove saved run?",
    exact: true,
  });
  await expect(removal).toContainText(
    "saved messages, output and account attempts",
  );
  await removal.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await page.evaluate(() => (window as any).routerRunCalls.length)).toBe(
    3,
  );
  await dialog
    .getByRole("button", { name: "Remove from history", exact: true })
    .click();
  await removal
    .getByRole("button", { name: "Remove from history", exact: true })
    .click();
  await expect(
    dialog
      .getByRole("combobox", { name: "Run history", exact: true })
      .locator("option"),
  ).toHaveCount(0);
  const removed = await page.evaluate(() => (window as any).routerRunCalls[3]);
  expect(removed).toMatchObject({
    command: "cli_run_remove",
    runId: await page.evaluate(
      () => (window as any).routerRunCalls[0].requestId,
    ),
    expectedRevision: 3,
  });
  expect(removed.requestId).toEqual(expect.any(String));
});

test("Codex pool opens a routed terminal without starting a managed run", async ({
  page,
}) => {
  await mockDesktop(page, false, undefined, undefined, undefined, undefined, {
    installed: [{ cli: "codex", name: "Codex", command: "codex" }],
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot = {
      revision: 1,
      profiles: [
        {
          id: "first",
          cli: "codex",
          label: "Personal",
          enabled: true,
          revision: 1,
          authState: "ready",
          storageMode: "cli_managed",
        },
        {
          id: "second",
          cli: "codex",
          label: "Work",
          enabled: true,
          revision: 1,
          authState: "ready",
          storageMode: "cli_managed",
        },
      ],
      routers: [
        {
          id: "codex-pool",
          cli: "codex",
          label: "Codex ordered pool",
          enabled: true,
          orderedProfileIds: ["second", "first"],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      capabilities: [
        {
          cli: "codex",
          name: "Codex",
          canStart: true,
          managedTurns: false,
          reason:
            "Account order chooses a new terminal only. Running CLI sessions keep their accounts.",
        },
      ],
      quota: [],
      runs: [],
    };
    desktop.routerTerminalCalls = [];
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_router_open_terminal") {
        desktop.routerTerminalCalls.push({ command, ...args });
        await desktop.__nativeTest.emitEvent("cli-router-open-profile", {
          cli: "codex",
          routerLabel: "Codex ordered pool",
          routerId: args.routerId,
          cwd: args.cwd,
          shellProfileId: args.shellProfileId,
        });
        return structuredClone(snapshot);
      }
      if (
        command === "start_terminal" &&
        args.request.routerId === "codex-pool"
      ) {
        const started = await original(command, args);
        return {
          ...started,
          routerProfileId: "second",
          routerProfileLabel: "Work",
        };
      }
      if (command === "cli_run_start") {
        desktop.routerTerminalCalls.push({ command, ...args });
        throw new Error("Codex managed turns unavailable");
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Agents", exact: true })
    .getByRole("button", { name: "Open router", exact: true })
    .click();
  const dialog = page.locator("dialog.router-launch-dialog");
  await chooseRoute(dialog, "codex-pool");
  await expect(dialog.getByLabel("Model", { exact: true })).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toHaveCount(0);
  await expect(
    dialog.getByText(
      "Uses the next routed account. Existing sessions keep their accounts.",
      { exact: false },
    ),
  ).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).routerTerminalCalls),
  ).toEqual([]);
  await page.screenshot({ path: "test-results/router-terminal.png" });
  await dialog
    .getByRole("button", { name: "Open routed terminal", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("tab", { name: "Codex · Codex ordered pool", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls
          .filter(
            (call: any) =>
              call.command === "start_terminal" &&
              call.args.request.routerId === "codex-pool",
          )
          .map((call: any) => ({
            profileId: call.args.request.profileId,
            routerProfileId: call.args.request.routerProfileId,
            routerId: call.args.request.routerId,
            cwd: call.args.request.cwd,
            cliLaunch: call.args.request.cliLaunch,
          })),
      ),
    )
    .toEqual([
      {
        profileId: "local:bash",
        routerProfileId: null,
        routerId: "codex-pool",
        cwd: "/project",
        cliLaunch: "codex",
      },
    ]);
  await expect(
    page.locator(".terminal-pane:visible .terminal-title"),
  ).toContainText("Work");
  expect(
    await page.evaluate(() => (window as any).routerTerminalCalls),
  ).toEqual([
    {
      command: "cli_router_open_terminal",
      routerId: "codex-pool",
      cwd: "/project",
      shellProfileId: "local:bash",
    },
  ]);
});

test("CLI close leases fence commands through Chat and Android cleanup", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.goto("/?window=settings&page=agent-control");
  const outcomes = await page.evaluate(async () => {
    const runtimePath = "/src/router/run-runtime.ts";
    const modelPath = "/src/model.ts";
    const chatPath = "/src/chat/chat-service.ts";
    const androidPath = "/src/android/service.ts";
    const runtime = await import(runtimePath);
    const model = await import(modelPath);
    const chat = await import(chatPath);
    const android = await import(androidPath);
    await android.androidRuntime();
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot = {
      revision: 1,
      profiles: [],
      routers: [],
      quota: [],
      capabilities: [],
      runs: [{ id: "closing-run", state: "completed" }],
    };
    const results = [];
    for (const cancel of [true, false]) {
      const project = model.newProject("/project", "local:bash");
      const workspace = project.workspaces[0];
      const cli = model.newCliAgentTab("closing-run", "CLI");
      const conversation = model.newChatTab("closing-chat", "Chat");
      const phone = model.newAndroidTab("closing-phone", "Phone");
      workspace.tabs = [cli, conversation, phone];
      workspace.activeTabId = cli.id;
      const session = {
        ...model.newSession(),
        projects: [project],
        activeProjectId: project.id,
      };
      const ids = new Set<string>(workspace.tabs.map((tab: any) => tab.id));
      const calls: string[] = [];
      let finishPin!: () => void;
      let finishChat!: () => void;
      let finishAndroid!: () => void;
      let chatStarted!: () => void;
      let androidStarted!: () => void;
      const chatEntered = new Promise<void>((resolve) => {
        chatStarted = resolve;
      });
      const androidEntered = new Promise<void>((resolve) => {
        androidStarted = resolve;
      });
      desktop.__TAURI_INTERNALS__.invoke = async (
        command: string,
        args: any,
      ) => {
        if (command === "cli_router_snapshot") return snapshot;
        if (command.startsWith("cli_run_")) {
          calls.push(command);
          if (command === "cli_run_pin")
            await new Promise<void>((resolve) => {
              finishPin = resolve;
            });
          return snapshot;
        }
        if (command === "chat_close") {
          chatStarted();
          await new Promise<void>((resolve) => {
            finishChat = resolve;
          });
          return;
        }
        if (command === "android_stop") {
          androidStarted();
          await new Promise<void>((resolve) => {
            finishAndroid = resolve;
          });
          return;
        }
        if (command === "android_state")
          return { devices: [], statuses: [], streams: [] };
        return original(command, args);
      };
      runtime.retainCliAgents(session);
      chat.retainChats(session);
      android.retainAndroid(session);
      const view = runtime.getRunView(cli.runId);
      const pin = view.command("cli_run_pin", { runId: cli.runId });
      const lease = await runtime.closeCliAgentViews(ids);
      finishPin();
      await pin;
      const chatClose = chat.closeChatViews(ids);
      await chatEntered;
      await view.command("cli_run_send", { runId: cli.runId });
      const chatFenced = view.state.busy && !calls.includes("cli_run_send");
      finishChat();
      await chatClose;
      const androidClose = android.stopLastAndroidViews(ids, () => session);
      await androidEntered;
      runtime.retainCliAgents({
        ...session,
        sidebarWidth: session.sidebarWidth + 1,
      });
      await view.command("cli_run_send", { runId: cli.runId });
      const androidFenced = view.state.busy && !calls.includes("cli_run_send");
      finishAndroid();
      await androidClose;
      if (cancel) {
        await lease.release();
        await view.command("cli_run_send", { runId: cli.runId });
      } else {
        runtime.retainCliAgents(undefined);
        await lease.release();
      }
      results.push({
        cancel,
        chatFenced,
        androidFenced,
        calls,
        unlocked: !view.state.busy,
      });
      runtime.retainCliAgents(undefined);
    }
    desktop.__TAURI_INTERNALS__.invoke = original;
    return results;
  });
  expect(outcomes).toEqual([
    {
      cancel: true,
      chatFenced: true,
      androidFenced: true,
      unlocked: true,
      calls: [
        "cli_run_pin",
        "cli_run_drain",
        "cli_run_close_release",
        "cli_run_send",
      ],
    },
    {
      cancel: false,
      chatFenced: true,
      androidFenced: true,
      unlocked: false,
      calls: ["cli_run_pin", "cli_run_drain", "cli_run_close_release"],
    },
  ]);
});

test("quota expires locally without polling native snapshots", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.clock.install({ time: new Date("2026-10-03T10:00:00Z") });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const timestamp = Date.now();
    const snapshot = {
      revision: 1,
      capabilities: [
        {
          cli: "codex",
          name: "Codex",
          canCreateProfile: true,
          canStart: true,
          managedTurns: false,
          quotaRead: true,
          balance: true,
          reason: "Separate account terminals and quota are available.",
        },
      ],
      profiles: ["first", "second"].map((id) => ({
        id,
        cli: "codex",
        label: id,
        enabled: true,
        revision: 1,
        authState: "ready",
        storageMode: "cli_managed",
      })),
      routers: [
        {
          id: "pool",
          cli: "codex",
          label: "Balanced pool",
          enabled: true,
          orderedProfileIds: ["first", "second"],
          balanceRemainingQuota: true,
          revision: 1,
        },
      ],
      quota: ["first", "second"].map((profileId) => ({
        profileId,
        status: "fresh",
        observedAt: timestamp,
        expiresAt: timestamp + 2000,
        epoch: 5,
        blockRevision: 0,
        windows: [
          { id: "short", remainingPercent: 80, resetAt: null },
          { id: "long", remainingPercent: 40, resetAt: null },
        ],
      })),
      runs: [],
    };
    desktop.quotaSnapshotReads = 0;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") {
        desktop.quotaSnapshotReads++;
        return structuredClone(snapshot);
      }
      return original(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
  await chooseCli(page, "codex");
  await expect(page.getByLabel("40% remaining", { exact: true })).toHaveCount(
    2,
  );
  await expect(page.getByText("Account order", { exact: true })).toHaveCount(0);
  const reads = await page.evaluate(() => (window as any).quotaSnapshotReads);
  await page.clock.fastForward(2001);
  await expect(page.getByLabel("Quota stale", { exact: true })).toHaveCount(2);
  await expect(page.getByText("Account order", { exact: true })).toBeVisible();
  await page.clock.fastForward(60000);
  expect(await page.evaluate(() => (window as any).quotaSnapshotReads)).toBe(
    reads,
  );
});

test("API-only profiles require a key and never open a login terminal", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot = {
      revision: 1,
      capabilities: [
        {
          cli: "openclaw",
          name: "OpenClaw",
          canCreateProfile: true,
          canVerifyLogin: false,
          profileTerminal: false,
          canStart: true,
          managedTurns: true,
          apiKeyLabel: "OpenAI API key",
          sameAccountResume: false,
          crossAccountResume: false,
          quotaRead: false,
          balance: false,
          readOnly: false,
          reason: "API-only text turns; ordinary profile login is unavailable.",
        },
      ],
      profiles: [
        {
          id: "account",
          cli: "openclaw",
          label: "API account",
          enabled: true,
          revision: 1,
          authState: "disconnected",
          storageMode: "cli_managed",
        },
      ],
      routers: [],
      quota: [],
      runs: [],
    };
    desktop.routerProfileCommands = [];
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_profile_set_api_key") {
        desktop.routerProfileCommands.push(command);
        Object.assign(snapshot.profiles[0], {
          authState: "ready",
          storageMode: "api_key",
          revision: 2,
        });
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      if (command === "cli_profile_open_terminal") {
        desktop.routerProfileCommands.push(command);
        throw new Error("An isolated profile terminal is unavailable.");
      }
      return original(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
  await chooseCli(page, "openclaw");
  await page
    .getByRole("button", { name: "Manage API account", exact: true })
    .click();
  const login = page.getByRole("button", { name: "Sign in with CLI" });
  await expect(login).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Verify", exact: true }),
  ).toBeDisabled();
  const key = page.getByLabel("OpenAI API key", { exact: true });
  await key.fill("fixture-api-key-for-source-regression");
  await page
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "API account", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Manage API account", exact: true })
    .click();
  await expect(key).toHaveValue("");
  await expect(
    page.getByRole("button", { name: "Verify", exact: true }),
  ).toBeEnabled();
  await expect(login).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).routerProfileCommands),
  ).toEqual(["cli_profile_set_api_key"]);
});

test("restored CLI views share a draft and drain only their final reference", async ({
  page,
}) => {
  const { newProject, newSession, newCliAgentTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const workspace = project.workspaces[0];
  const first = newCliAgentTab("retained-run", "First saved");
  const second = newCliAgentTab("retained-run", "Second saved");
  workspace.tabs = [first, second];
  workspace.activeTabId = first.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot = {
      revision: 5,
      profiles: [],
      quota: [],
      routers: [
        {
          id: "pool",
          cli: "gemini",
          label: "Saved pool",
          enabled: true,
          orderedProfileIds: [],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      capabilities: [{ cli: "gemini", name: "Gemini CLI", managedTurns: true }],
      runs: [
        {
          id: "retained-run",
          routerId: "pool",
          cwd: "/project",
          shellProfileId: null,
          title: "Saved task",
          state: "completed",
          model: "gemini-model",
          pinnedProfileId: null,
          allowedProfileIds: [],
          activeProfileId: null,
          generation: 1,
          revision: 5,
          inputs: [{ id: "input", text: "Saved question" }],
          attempts: [],
          turns: [
            {
              inputId: "input",
              attemptId: "attempt",
              profileId: "first",
              generation: 1,
              state: "completed",
              text: "Saved answer",
            },
          ],
          legacyOutput: null,
          output: "Saved answer",
          statusMessage: "",
          attemptedProfileIds: [],
        },
      ],
    };
    desktop.savedRunCalls = [];
    desktop.failRunDrain = true;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_run_close_release") return;
      if (
        ["cli_run_start", "cli_run_send", "cli_run_drain"].includes(command)
      ) {
        desktop.savedRunCalls.push({ command, ...args });
        if (command !== "cli_run_drain")
          throw new Error("Restoration must not dispatch work");
        if (desktop.failRunDrain)
          throw new Error("Cleanup could not be confirmed");
        return structuredClone(snapshot);
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  const firstPane = page.getByRole("region", {
    name: "First saved",
    exact: true,
  });
  await expect(
    firstPane.getByText("Saved answer", { exact: true }),
  ).toBeVisible();
  await firstPane
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Draft stays with saved run");
  await page.getByRole("tab", { name: "Second saved", exact: true }).click();
  const secondPane = page.getByRole("region", {
    name: "Second saved",
    exact: true,
  });
  await expect(
    secondPane.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveValue("Draft stays with saved run");
  expect(await page.evaluate(() => (window as any).savedRunCalls)).toEqual([]);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ),
    ),
  ).toEqual([]);
  await page
    .getByRole("button", { name: "Close First saved", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "First saved", exact: true }),
  ).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).savedRunCalls)).toEqual([]);
  await page
    .getByRole("button", { name: "Close Second saved", exact: true })
    .click();
  const failure = page.getByRole("dialog", {
    name: "Views could not be closed",
    exact: true,
  });
  await expect(failure).toContainText("Cleanup could not be confirmed");
  await expect(
    page.getByRole("tab", { name: "Second saved", exact: true }),
  ).toBeVisible();
  await failure.getByRole("button", { name: "Keep open", exact: true }).click();
  await page.evaluate(() => {
    (window as any).failRunDrain = false;
  });
  await page
    .getByRole("button", { name: "Close Second saved", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "Second saved", exact: true }),
  ).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).savedRunCalls)).toEqual([
    { command: "cli_run_drain", runId: "retained-run" },
    { command: "cli_run_drain", runId: "retained-run" },
  ]);
});

test("account terminal opening aborts when its workspace disappears during snapshot read", async ({
  page,
}) => {
  const { newProject, newSession, newBrowserTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const workspace = project.workspaces[0];
  const browser = newBrowserTab();
  workspace.tabs = [browser];
  workspace.activeTabId = browser.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") {
        desktop.accountSnapshotPending = true;
        return new Promise((resolve) => {
          desktop.finishAccountSnapshot = () =>
            resolve({
              revision: 1,
              routers: [],
              quota: [],
              capabilities: [],
              runs: [],
              profiles: [
                {
                  id: "account",
                  cli: "codex",
                  label: "Account",
                  enabled: true,
                },
              ],
            });
        });
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Toggle workspaces", exact: true })
    .click();
  const target = page.locator(".workspace-list").getByRole("button", {
    name: new RegExp(`^${workspace.name} `),
  });
  await expect(target).toBeVisible();
  await page.evaluate(async () => {
    await (window as any).__nativeTest.emitEvent("cli-router-open-profile", {
      profileId: "account",
      cwd: "/project",
      shellProfileId: "local:bash",
    });
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).accountSnapshotPending))
    .toBe(true);
  await target.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: "Delete workspace…", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Delete workspace", exact: true })
    .getByRole("button", { name: "Continue", exact: true })
    .click();
  await expect(target).toHaveCount(0);
  await page.evaluate(() => (window as any).finishAccountSnapshot());
  await expect(
    page.getByText(
      "The workspace changed before the router terminal opened. Open it again from its project.",
      { exact: true },
    ),
  ).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" &&
          call.args?.request?.routerProfileId === "account",
      ),
    ),
  ).toEqual([]);
});

// Fixture catalogs intentionally differ per model: the webview must consume
// native capabilities rather than supplying an independent model/effort list.
async function nativeModelFixture(page: Page) {
  await mockDesktop(page, false, undefined, undefined, undefined, undefined, {
    installed: [{ cli: "codex", name: "Codex", command: "codex" }],
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot: any = {
      revision: 1,
      capabilities: [
        {
          cli: "codex",
          name: "Codex",
          canCreateProfile: true,
          canVerifyLogin: true,
          profileTerminal: true,
          canStart: true,
          managedTurns: true,
          quotaRead: true,
          balance: true,
          apiKeyLabel: null,
          reason: "Verified subscription text turns.",
          models: [
            { id: "gpt-6-sol", reasoningEfforts: ["medium", "high"] },
            { id: "gpt-6-luna", reasoningEfforts: ["low"] },
          ],
        },
        {
          cli: "openclaw",
          name: "OpenClaw",
          canCreateProfile: true,
          profileTerminal: false,
          canStart: true,
          managedTurns: true,
          apiKeyLabel: "OpenAI API key",
          models: [{ id: "gpt-4.1" }],
        },
        {
          cli: "hermes",
          name: "Hermes Agent",
          canCreateProfile: true,
          profileTerminal: false,
          canStart: true,
          managedTurns: true,
          apiKeyLabel: "OpenAI API key",
        },
      ],
      profiles: [
        {
          id: "subscription",
          cli: "codex",
          label: "Subscription account",
          enabled: true,
          revision: 1,
          authState: "ready",
          storageMode: "cli_managed",
        },
      ],
      routers: ["codex", "openclaw", "hermes"].map((cli) => ({
        id: `${cli}-pool`,
        cli,
        label: `${cli} router`,
        enabled: true,
        orderedProfileIds: [],
        balanceRemainingQuota: false,
        revision: 1,
      })),
      quota: [],
      runs: [],
    };
    desktop.nativeModelStartCalls = [];
    desktop.replaceNativeModels = async (cli: string, models: any[]) => {
      snapshot.capabilities.find((item: any) => item.cli === cli).models =
        models;
      snapshot.revision++;
      await desktop.__nativeTest.emitEvent("cli-router-changed", {
        revision: snapshot.revision,
      });
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_run_start") {
        desktop.nativeModelStartCalls.push(args.request);
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      return original(command, args);
    };
  });
}

// Exact implicit-label text includes descendant select options and textarea
// values. Accessible role names identify the control without those contents.
function runField(scope: Locator, name: string) {
  return scope
    .getByRole("combobox", { name, exact: true })
    .or(scope.getByRole("textbox", { name, exact: true }));
}

async function openNativeModelDialog(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Agents", exact: true })
    .getByRole("button", { name: "Open router", exact: true })
    .click();
  return page.locator("dialog.router-launch-dialog");
}

test("new run and history stay separate and optional setup stays collapsed", async ({
  page,
}) => {
  await nativeModelFixture(page);
  const dialog = await openNativeModelDialog(page);
  await expect(dialog.getByRole("radio")).toHaveCount(3);
  await expect(
    dialog.getByRole("combobox", { name: "Run history", exact: true }),
  ).toHaveCount(0);
  await expect(runField(dialog, "Model")).toHaveCount(0);
  await chooseRoute(dialog, "codex-pool");
  await expect(runField(dialog, "Model")).toBeVisible();
  await expect(
    dialog.getByRole("textbox", { name: "Run name", exact: true }),
  ).not.toBeVisible();
  await dialog.getByText("Name this run", { exact: true }).click();
  await expect(
    dialog.getByRole("textbox", { name: "Run name", exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: /^History/ }).click();
  if (await dialog.getByText("All runs", { exact: true }).isVisible())
    await dialog.getByText("All runs", { exact: true }).click();
  await expect(
    dialog.getByText("No saved runs yet", { exact: true }),
  ).toBeVisible();
  await expect(runField(dialog, "Model")).toHaveCount(0);
  await dialog
    .getByRole("button", { name: "New run", exact: true })
    .first()
    .click();
  await chooseRoute(dialog, "codex-pool");
  await expect(runField(dialog, "Model")).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await page.screenshot({ path: "test-results/router-ux-new-run.png" });
  await page.setViewportSize({ width: 800, height: 600 });
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      dialog
        .getByRole("button", { name: "Start run", exact: true })
        .evaluate((button) => {
          const bounds = button.getBoundingClientRect();
          const modal = button.closest("dialog")!.getBoundingClientRect();
          return bounds.bottom <= modal.bottom && bounds.top >= modal.top;
        }),
    )
    .toBe(true);
  await page.screenshot({ path: "test-results/router-ux-new-run-compact.png" });
});

test("account details open with the keyboard and setup never starts CLI work", async ({
  page,
}) => {
  await nativeModelFixture(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    let created: any;
    desktop.uxSetupCalls = [];
    async function snapshot() {
      const result = await original("cli_router_snapshot");
      if (created) {
        result.profiles.push(created);
        result.revision = 2;
      }
      return result;
    }
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return snapshot();
      if (command === "cli_profile_native_report")
        return args.profileId === "subscription"
          ? {
              profileId: "subscription",
              profileRevision: 1,
              observedAt: Date.now(),
              expiresAt: Date.now() + 30000,
              authenticated: true,
              observation: {
                source: "fixture",
                identity: { displayLabel: "Personal account" },
                windows: [
                  {
                    id: "hourly",
                    name: "Hourly",
                    nativeWindow: "5-hour window",
                    remainingPercent: 80,
                    disabled: false,
                    resetAt: null,
                  },
                  {
                    id: "weekly",
                    name: "Weekly",
                    nativeWindow: "Weekly window",
                    remainingPercent: null,
                    disabled: false,
                    resetAt: null,
                  },
                ],
              },
            }
          : null;
      if (command === "cli_router_mutate") {
        desktop.uxSetupCalls.push({ command, ...args.request });
        if (
          args.request.expectedRevision !== 1 ||
          args.request.action.type !== "create_profile"
        )
          throw new Error("Unexpected setup mutation");
        created = {
          id: "new-account",
          cli: "codex",
          label: args.request.action.label,
          enabled: true,
          revision: 1,
          authState: "disconnected",
          storageMode: "cli_managed",
        };
        return snapshot();
      }
      if (
        [
          "cli_run_start",
          "cli_run_send",
          "cli_profile_open_terminal",
          "cli_profile_verify",
        ].includes(command)
      ) {
        desktop.uxSetupCalls.push({ command });
        throw new Error("Setup must not start CLI work");
      }
      return original(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
  await chooseCli(page, "codex");
  await expect(
    page.getByRole("button", { name: "Sign in with CLI", exact: true }),
  ).not.toBeVisible();
  await expect(
    page.getByText("Verified subscription text turns.", { exact: true }),
  ).not.toBeVisible();
  await page.screenshot({ path: "test-results/router-ux-settings.png" });
  await page
    .getByRole("button", { name: "Manage Subscription account", exact: true })
    .click();
  const detailsDialog = page.getByRole("dialog", {
    name: "Subscription account",
    exact: true,
  });
  const details = detailsDialog
    .getByText("Account details", { exact: true })
    .locator("..");
  await expect(
    detailsDialog.getByText("80.0% remaining", { exact: true }),
  ).not.toBeVisible();
  await details.focus();
  await details.press("Enter");
  await expect(
    detailsDialog.getByText("80.0% remaining", { exact: true }),
  ).toBeVisible();
  await expect(
    detailsDialog.getByText("Quota unknown", { exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: "test-results/router-style-account-details-light.png",
  });
  await page.setViewportSize({ width: 800, height: 600 });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await expect
    .poll(() =>
      detailsDialog
        .getByRole("button", { name: "Done", exact: true })
        .evaluate((button) => {
          const bounds = button.getBoundingClientRect();
          const modal = button.closest("dialog")!.getBoundingClientRect();
          return bounds.bottom <= modal.bottom && bounds.top >= modal.top;
        }),
    )
    .toBe(true);
  await page.screenshot({
    path: "test-results/router-style-account-details-dark.png",
  });
  await details.press("Space");
  await expect(
    detailsDialog.getByText("80.0% remaining", { exact: true }),
  ).not.toBeVisible();
  expect(await page.evaluate(() => (window as any).uxSetupCalls)).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(detailsDialog).toHaveCount(0);
  await page.emulateMedia({ colorScheme: "light" });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("button", { name: "Add account", exact: true }).click();
  const add = page.getByRole("dialog", { name: "Add account", exact: true });
  const name = add.getByRole("textbox", { name: "Account name", exact: true });
  await expect(name).toBeFocused();
  await name.fill("Work");
  await add.getByRole("button", { name: "Add account", exact: true }).click();
  const account = page.getByRole("dialog", { name: "Work", exact: true });
  await expect(
    account.getByRole("button", { name: "Sign in with CLI", exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/router-ux-account.png" });
  await page.setViewportSize({ width: 800, height: 600 });
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-appearance", "dark");
  await page.screenshot({
    path: "test-results/router-ux-account-compact-dark.png",
  });
  await page.keyboard.press("Escape");
  await expect(account).toHaveCount(0);
  await page.screenshot({
    path: "test-results/router-ux-settings-compact-dark.png",
  });
  expect(await page.evaluate(() => (window as any).uxSetupCalls)).toEqual([
    expect.objectContaining({
      command: "cli_router_mutate",
      expectedRevision: 1,
      action: { type: "create_profile", cli: "codex", label: "Work" },
    }),
  ]);
});

test("changing how a run works clears the previous model and thinking choice", async ({
  page,
}) => {
  await nativeModelFixture(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const result = await original(command, args);
      if (result?.capabilities)
        result.capabilities.find(
          (item: any) => item.cli === "codex",
        ).codingTurns = true;
      return result;
    };
  });
  const dialog = await openNativeModelDialog(page);
  await chooseRoute(dialog, "codex-pool");
  const mode = dialog.getByRole("radiogroup", {
    name: "Execution mode",
    exact: true,
  });
  const model = runField(dialog, "Model");
  const effort = runField(dialog, "Reasoning effort");
  await chooseMode(dialog, "coding");
  await model.selectOption("gpt-6-sol");
  await effort.selectOption("high");
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toBeEnabled();
  await chooseMode(dialog, "text");
  await expect(model).toHaveValue("");
  await expect(effort).toHaveValue("");
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toBeDisabled();
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
});

test("native model catalogs require explicit choices and fence efforts across CLI changes", async ({
  page,
}) => {
  await nativeModelFixture(page);
  const dialog = await openNativeModelDialog(page);

  const model = runField(dialog, "Model");
  const effort = runField(dialog, "Reasoning effort");
  const start = dialog.getByRole("button", { name: "Start run", exact: true });
  await chooseRoute(dialog, "codex-pool");
  await expect(model).toHaveValue("");
  await expect(model.locator("option")).toHaveText([
    "Choose a model",
    "gpt-6-sol",
    "gpt-6-luna",
  ]);
  await expect(effort).toBeDisabled();
  await expect(start).toBeDisabled();
  await model.selectOption("gpt-6-sol");
  await expect(effort).toHaveValue("");
  await expect(effort.locator("option")).toHaveText([
    "Choose reasoning effort",
    "medium",
    "high",
  ]);
  await expect(start).toBeDisabled();
  await effort.selectOption("high");
  await expect(start).toBeEnabled();
  await dialog.getByRole("button", { name: "Back", exact: true }).click();
  await dialog.getByRole("button", { name: "Next", exact: true }).click();
  await expect(model).toHaveValue("gpt-6-sol");
  await expect(effort).toHaveValue("high");
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await model.selectOption("gpt-6-luna");
  await expect(effort).toHaveValue("");
  await expect(effort.locator("option")).toHaveText([
    "Choose reasoning effort",
    "low",
  ]);
  await expect(start).toBeDisabled();
  await effort.selectOption("low");
  await chooseRoute(dialog, "openclaw-pool");
  await expect(model).toHaveValue("");
  await expect(model.locator("option")).toHaveText([
    "Choose a model",
    "gpt-4.1",
  ]);
  await expect(effort).toHaveCount(0);
  await expect(start).toBeDisabled();
  await model.selectOption("gpt-4.1");
  await expect(start).toBeEnabled();
  await chooseRoute(dialog, "codex-pool");
  await expect(model).toHaveValue("");
  await expect(effort).toHaveValue("");
  await expect(start).toBeDisabled();
  await model.selectOption("gpt-6-sol");
  await effort.selectOption("medium");
  await start.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).nativeModelStartCalls))
    .toEqual([
      expect.objectContaining({
        routerId: "codex-pool",
        model: "gpt-6-sol",
        reasoningEffort: "medium",
      }),
    ]);
  await chooseRoute(dialog, "hermes-pool");
  await expect(model).toHaveValue("");
  expect(await model.evaluate((element) => element.tagName)).toBe("INPUT");
  await model.fill("provider-model-without-native-fixed-catalog");
  await expect(start).toBeEnabled();
});

test("updated native catalog rejects stale model and effort even on direct form submission", async ({
  page,
}) => {
  await nativeModelFixture(page);
  const dialog = await openNativeModelDialog(page);
  await chooseRoute(dialog, "codex-pool");
  const model = runField(dialog, "Model");
  const effort = runField(dialog, "Reasoning effort");
  const start = dialog.getByRole("button", { name: "Start run", exact: true });
  const submit = async () =>
    model.evaluate((element) => {
      element
        .closest("form")!
        .dispatchEvent(
          new Event("submit", { bubbles: true, cancelable: true }),
        );
    });
  await model.selectOption("gpt-6-sol");
  await effort.selectOption("high");
  await page.evaluate(() =>
    (window as any).replaceNativeModels("codex", [
      { id: "gpt-6-sol", reasoningEfforts: ["medium"] },
      { id: "gpt-6-luna", reasoningEfforts: ["low"] },
    ]),
  );
  await expect(effort.locator("option")).toHaveText([
    "Choose reasoning effort",
    "medium",
  ]);
  await expect(start).toBeDisabled();
  await submit();
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await effort.selectOption("medium");
  await expect(start).toBeEnabled();
  await page.evaluate(() =>
    (window as any).replaceNativeModels("codex", [
      { id: "gpt-6-luna", reasoningEfforts: ["low"] },
    ]),
  );
  await expect(model.locator("option")).toHaveText([
    "Choose a model",
    "gpt-6-luna",
  ]);
  await expect(start).toBeDisabled();
  await submit();
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await model.selectOption("gpt-6-luna");
  await effort.selectOption("low");
  await start.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).nativeModelStartCalls))
    .toEqual([
      expect.objectContaining({ model: "gpt-6-luna", reasoningEffort: "low" }),
    ]);
});

test("managed Codex accounts show subscription guidance without an API key form", async ({
  page,
}) => {
  await nativeModelFixture(page);
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
  await chooseCli(page, "codex");
  await page
    .getByRole("button", { name: "Manage Subscription account", exact: true })
    .click();
  const account = page.getByRole("dialog", {
    name: "Subscription account",
    exact: true,
  });
  await expect(
    account.getByRole("button", { name: "Sign in with CLI", exact: true }),
  ).toBeEnabled();
  await expect(
    account.getByRole("button", { name: "Verify", exact: true }),
  ).toBeEnabled();
  await expect(
    account.getByRole("textbox", { name: "API key", exact: true }),
  ).toHaveCount(0);
  await expect(account.locator('input[type="password"]')).toHaveCount(0);
});

test("coding requires native capability and preserves exact model choices without auto-start", async ({
  page,
}) => {
  await nativeModelFixture(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    let coding = true;
    let revision = 0;
    desktop.setCodingAvailable = async (available: boolean) => {
      coding = available;
      revision++;
      await desktop.__nativeTest.emitEvent("cli-router-changed", {
        revision: 100 + revision,
      });
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const result = await original(command, args);
      if (result?.capabilities) {
        result.revision += revision;
        result.capabilities.find(
          (item: any) => item.cli === "codex",
        ).codingTurns = coding;
      }
      return result;
    };
  });
  const dialog = await openNativeModelDialog(page);
  await chooseRoute(dialog, "codex-pool");
  const mode = dialog.getByRole("radiogroup", {
    name: "Execution mode",
    exact: true,
  });
  const model = runField(dialog, "Model");
  const effort = runField(dialog, "Reasoning effort");
  const start = dialog.getByRole("button", { name: "Start run", exact: true });
  await expect(mode.locator("input:checked")).toHaveCount(0);
  await chooseMode(dialog, "coding");
  await expect(model).toHaveValue("");
  await expect(start).toBeDisabled();
  await expect(dialog.locator(".router-launch-selection")).toContainText(
    "Edit files",
  );
  await model.selectOption("gpt-6-sol");
  await effort.selectOption("high");
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await page.evaluate(() => (window as any).setCodingAvailable(false));
  await expect(mode.locator('input[value="coding"]')).toHaveCount(0);
  await expect(mode.locator('input[value="text"]')).toBeChecked();
  await expect(model).toHaveCount(0);
  await expect(effort).toHaveCount(0);
  await mode.evaluate((element) =>
    element
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await page.evaluate(() => (window as any).setCodingAvailable(true));
  await expect(mode).toBeVisible();
  await expect(mode.locator("input:checked")).toHaveCount(0);
  await chooseMode(dialog, "coding");
  await expect(model).toHaveValue("");
  await expect(effort).toHaveValue("");
  await expect(start).toBeDisabled();
  await model.selectOption("gpt-6-sol");
  await effort.selectOption("high");
  await start.click();
  await expect
    .poll(() => page.evaluate(() => (window as any).nativeModelStartCalls))
    .toEqual([
      expect.objectContaining({
        executionMode: "coding",
        model: "gpt-6-sol",
        reasoningEffort: "high",
      }),
    ]);
  await chooseRoute(dialog, "openclaw-pool");
  await expect(model).toHaveValue("");
  await dialog.getByRole("button", { name: "Back", exact: true }).click();
  await expect(mode.getByRole("radio")).toHaveCount(1);
  await expect(
    mode.getByRole("radio", { name: "Chat", exact: true }),
  ).toBeChecked();
});

test("coding pane acknowledgement never launches and UTF-8 overflow retains the draft", async ({
  page,
}) => {
  const { newProject, newSession, newCliAgentTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const tab = newCliAgentTab("coding-run", "Coding task");
  project.workspaces[0].tabs = [tab];
  project.workspaces[0].activeTabId = tab.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const run = {
      id: "coding-run",
      routerId: "pool",
      cwd: "/project",
      shellProfileId: null,
      title: "Coding task",
      state: "recovery_required",
      model: "gpt-6-sol",
      reasoningEffort: "high",
      executionMode: "coding",
      pinnedProfileId: null,
      allowedProfileIds: [],
      activeProfileId: null,
      generation: 2,
      revision: 7,
      inputs: [{ id: "input", text: "Original assignment" }],
      attempts: [],
      turns: [],
      output: "",
      statusMessage: "Drain was uncertain",
      attemptedProfileIds: [],
    };
    const snapshot = {
      revision: 1,
      profiles: [],
      quota: [],
      routers: [
        {
          id: "pool",
          cli: "codex",
          label: "Codex",
          enabled: true,
          orderedProfileIds: [],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      capabilities: [
        { cli: "codex", name: "Codex", managedTurns: true, codingTurns: true },
      ],
      runs: [run],
    };
    desktop.codingCalls = [];
    desktop.refuseAcknowledgement = true;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_run_effect_approvals") {
        desktop.codingApprovalReads = (desktop.codingApprovalReads ?? 0) + 1;
        return [];
      }
      if (command === "cli_run_acknowledge_coding_completion") {
        desktop.codingCalls.push({ command, request: args.request });
        if (desktop.refuseAcknowledgement)
          throw new Error(
            "RecoveryRequired: unknown dispatch has no checkpoint",
          );
        run.state = "idle";
        run.revision++;
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      if (command === "cli_run_send" || command === "cli_run_start") {
        desktop.codingCalls.push({ command, request: args.request });
        return structuredClone(snapshot);
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  const pane = page.locator(".cli-agent-pane");
  await expect
    .poll(() => page.evaluate(() => (window as any).codingApprovalReads ?? 0))
    .toBeGreaterThan(0);
  const acknowledge = pane.getByRole("button", {
    name: "Acknowledge recorded completion",
    exact: true,
  });
  await acknowledge.click();
  await expect(pane).toContainText("unknown dispatch has no checkpoint");
  await page.evaluate(() => {
    (window as any).refuseAcknowledgement = false;
  });
  await acknowledge.click();
  const message = pane.getByRole("textbox", { name: "Message", exact: true });
  await expect(message).toBeEnabled();
  const draft = "🦀".repeat(16000);
  await message.fill(draft);
  await expect(pane).toContainText("60 KiB UTF-8 limit");
  await expect(
    pane.getByRole("button", { name: "Send", exact: true }),
  ).toBeDisabled();
  await message.evaluate((element) =>
    element
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  await expect(message).toHaveValue(draft);
  const calls = await page.evaluate(() => (window as any).codingCalls);
  expect(calls).toHaveLength(2);
  expect(
    calls.every(
      (call: any) => call.command === "cli_run_acknowledge_coding_completion",
    ),
  ).toBe(true);
  expect(calls[0].request).toEqual(
    expect.objectContaining({ runId: "coding-run", expectedRunRevision: 7 }),
  );
});

test("gateway creation requires an explicit mode and arbitrary model without reasoning substitution", async ({
  page,
}) => {
  await nativeModelFixture(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const result = await original(command, args);
      if (result?.capabilities)
        result.capabilities.find(
          (item: any) => item.cli === "codex",
        ).gatewayTerminal = true;
      return result;
    };
  });
  const dialog = await openNativeModelDialog(page);
  await chooseRoute(dialog, "codex-pool");
  await chooseMode(dialog, "gateway");
  const model = runField(dialog, "Model");
  expect(await model.evaluate((element) => element.tagName)).toBe("INPUT");
  await expect(model).toHaveValue("");
  await expect(runField(dialog, "Reasoning effort")).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toBeDisabled();
  await model.fill("provider/exact-model-id");
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await dialog.getByRole("button", { name: "Start run", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).nativeModelStartCalls))
    .toEqual([
      expect.objectContaining({
        executionMode: "gateway",
        model: "provider/exact-model-id",
        reasoningEffort: null,
      }),
    ]);
});

test("restored Gateway pane waits for explicit Start CLI and binds the saved shell and revision", async ({
  page,
}) => {
  const { newProject, newSession, newCliAgentTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const tab = newCliAgentTab("gateway-run", "Gateway task");
  project.workspaces[0].tabs = [tab];
  project.workspaces[0].activeTabId = tab.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const run = {
      id: "gateway-run",
      routerId: "pool",
      cwd: "/project",
      shellProfileId: "local:bash",
      title: "Gateway task",
      state: "idle",
      model: "exact-model",
      reasoningEffort: null,
      executionMode: "gateway",
      pinnedProfileId: null,
      allowedProfileIds: [],
      activeProfileId: null,
      generation: 0,
      revision: 3,
      inputs: [],
      attempts: [],
      turns: [],
      output: "",
      statusMessage: "",
      attemptedProfileIds: [],
    };
    const snapshot = {
      revision: 1,
      profiles: [],
      quota: [],
      routers: [
        {
          id: "pool",
          cli: "codex",
          label: "Codex",
          enabled: true,
          orderedProfileIds: [],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      capabilities: [
        {
          cli: "codex",
          name: "Codex",
          gatewayTerminal: true,
          nativeHistoryResume: true,
          managedTurns: false,
        },
      ],
      runs: [run],
    };
    desktop.gatewayCalls = [];
    desktop.gatewayLongStatus = async () => {
      run.statusMessage = "Temporary gateway failure. ".repeat(30);
      snapshot.revision++;
      await desktop.__nativeTest.emitEvent("cli-router-changed", {
        revision: snapshot.revision,
      });
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "start_terminal") {
        desktop.gatewayCalls.push({ command, request: args.request });
        run.generation++;
        run.state = "running";
        run.revision++;
        snapshot.revision++;
        await desktop.__nativeTest.emitEvent("cli-router-changed", {
          revision: snapshot.revision,
        });
        return { cwd: "/project" };
      }
      if (command === "cli_run_send")
        throw new Error("Gateway cannot use text-turn send");
      if (command === "cli_run_stop") {
        desktop.gatewayCalls.push({ command });
        run.state = "stopped";
        run.revision++;
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  const pane = page.locator(".cli-agent-pane");
  await expect(
    pane.getByRole("button", { name: "Start CLI", exact: true }),
  ).toBeVisible();
  await expect(
    pane.getByRole("textbox", { name: "Message", exact: true }),
  ).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).gatewayCalls)).toEqual([]);
  await pane.getByRole("button", { name: "Start CLI", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).gatewayCalls))
    .toEqual([
      expect.objectContaining({
        command: "start_terminal",
        request: expect.objectContaining({
          profileId: "local:bash",
          cwd: "/project",
          cliLaunch: "codex",
          gatewayRunId: "gateway-run",
          gatewayRunRevision: 3,
          routerProfileId: null,
          routerId: null,
        }),
      }),
    ]);
  await pane.evaluate((element) => {
    Object.assign((element as HTMLElement).style, {
      height: "300px",
      flex: "none",
    });
  });
  await page.evaluate(() => (window as any).gatewayLongStatus());
  await expect(pane.locator(".router-gateway-content")).toContainText(
    "Temporary gateway failure.",
  );
  await expect
    .poll(() =>
      pane.evaluate((element) => {
        const parent = element.getBoundingClientRect();
        const controls = element
          .querySelector(".router-gateway-content")!
          .getBoundingClientRect();
        const terminal = element
          .querySelector(".terminal-pane")!
          .getBoundingClientRect();
        return (
          controls.bottom <= terminal.top + 1 &&
          terminal.bottom <= parent.bottom + 1 &&
          terminal.height > 60
        );
      }),
    )
    .toBe(true);
  await page.screenshot({ path: "test-results/router-ux-gateway-short.png" });
  await pane.getByRole("button", { name: "Stop CLI", exact: true }).click();
  await expect(
    pane.getByRole("button", { name: "Resume CLI", exact: true }),
  ).toBeVisible();
  const callsBeforeResume = await page.evaluate(
    () =>
      (window as any).gatewayCalls.filter(
        (call: any) => call.command === "start_terminal",
      ).length,
  );
  expect(callsBeforeResume).toBe(1);
  await pane.getByRole("button", { name: "Resume CLI", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as any).gatewayCalls.filter(
            (call: any) => call.command === "start_terminal",
          ).length,
      ),
    )
    .toBe(2);
});

async function gatewaySettingsFixture(page: Page) {
  await nativeModelFixture(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    let provider: { protocol: string; baseUrl: string } | null = null;
    let revision = 0;
    desktop.gatewaySettingsCalls = [];
    async function snapshot() {
      const result = await original("cli_router_snapshot");
      result.revision += revision;
      const capability = result.capabilities.find(
        (item: any) => item.cli === "codex",
      );
      capability.gatewayTerminal = true;
      capability.gatewayProtocol = "openai_responses";
      capability.managedTurns = false;
      const profile = result.profiles.find(
        (item: any) => item.id === "subscription",
      );
      profile.gatewayProvider = provider;
      profile.revision += revision;
      return result;
    }
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return snapshot();
      if (
        command === "cli_router_mutate" &&
        args.request.action.type === "configure_gateway_account"
      ) {
        const before = await snapshot();
        if (args.request.expectedRevision !== before.revision)
          throw new Error("Stale endpoint review");
        desktop.gatewaySettingsCalls.push({ command, request: args.request });
        const requested = args.request.action.provider;
        provider = requested
          ? {
              protocol: requested.protocol,
              baseUrl: requested.baseUrl.replace(/\/+$/, ""),
            }
          : null;
        revision++;
        return snapshot();
      }
      if (command === "cli_profile_set_api_key") {
        const before = await snapshot();
        if (!provider || args.expectedRevision !== before.profiles[0].revision)
          throw new Error("Save the reviewed endpoint first");
        desktop.gatewaySettingsCalls.push({
          command,
          profileId: args.profileId,
          expectedRevision: args.expectedRevision,
          key: args.key,
        });
        revision++;
        return snapshot();
      }
      return original(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
  await chooseCli(page, "codex");
  await page
    .getByRole("button", { name: "Manage Subscription account", exact: true })
    .click();
  await page.getByRole("button", { name: "API key", exact: true }).click();
}

test("gateway endpoint canonicalization enables key save and endpoint changes discard old key drafts", async ({
  page,
}) => {
  await gatewaySettingsFixture(page);
  const endpoint = page.getByLabel("API endpoint", {
    exact: true,
  });
  const saveEndpoint = page.getByRole("button", {
    name: "Save endpoint",
    exact: true,
  });
  await page
    .getByRole("dialog", { name: "Subscription account", exact: true })
    .getByText("Advanced", { exact: true })
    .click();
  await endpoint.fill("https://api.example.test/");
  await saveEndpoint.click();
  // The native snapshot owns the canonical endpoint; a stale slash-bearing
  // input draft must not prevent saving a key for that exact destination.
  await expect(endpoint).toHaveValue("https://api.example.test");
  const key = page.getByLabel("API key", { exact: true });
  const saveKey = page.getByRole("button", {
    name: "Save connection",
    exact: true,
  });
  await key.fill("owned-first-test-key");
  await expect(saveKey).toBeEnabled();
  await saveKey.click();
  await expect(
    page.getByRole("dialog", { name: "Subscription account", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Manage Subscription account", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Subscription account", exact: true })
    .getByText("Advanced", { exact: true })
    .click();
  await expect(key).toHaveValue("");
  await key.fill("draft-for-old-endpoint");
  await endpoint.fill("https://other.example.test/");
  // Endpoint-only save discards an existing key draft instead of binding it.
  await saveEndpoint.click();
  await expect(endpoint).toHaveValue("https://other.example.test");
  await expect(key).toHaveValue("");
  await expect(saveKey).toBeDisabled();
  await key.fill("owned-second-test-key");
  await expect(saveKey).toBeEnabled();
  await saveKey.click();
  await expect(
    page.getByRole("dialog", { name: "Subscription account", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Manage Subscription account", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Subscription account", exact: true })
    .getByText("Advanced", { exact: true })
    .click();
  const calls = await page.evaluate(() => (window as any).gatewaySettingsCalls);
  expect(calls.map((call: any) => call.command)).toEqual([
    "cli_router_mutate",
    "cli_profile_set_api_key",
    "cli_router_mutate",
    "cli_profile_set_api_key",
  ]);
  expect(calls[0].request).toEqual(
    expect.objectContaining({
      expectedRevision: 1,
      action: expect.objectContaining({
        provider: {
          protocol: "openai_responses",
          baseUrl: "https://api.example.test/",
        },
      }),
    }),
  );
  expect(calls[1]).toEqual(
    expect.objectContaining({
      expectedRevision: 2,
      key: "owned-first-test-key",
    }),
  );
  expect(calls[3]).toEqual(
    expect.objectContaining({
      expectedRevision: 4,
      key: "owned-second-test-key",
    }),
  );
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  expect(
    await page.evaluate(() =>
      (window as any).__nativeTest.calls.filter(
        (call: any) => call.command === "start_terminal",
      ),
    ),
  ).toEqual([]);
});

// Offline native fixtures exercise UI intent only. They never execute a CLI,
// contact a provider, or establish account/approval qualification.
test("native creation is explicit and preserves the CLI model without dispatch on selection", async ({
  page,
}) => {
  await nativeModelFixture(page);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const result = await original(command, args);
      if (result?.capabilities) {
        result.capabilities.push({
          cli: "claude",
          name: "Claude Code",
          nativeTurns: true,
          nativeAccountTerminal: true,
          managedTurns: false,
        });
        result.routers.push({
          id: "claude-native",
          cli: "claude",
          label: "Claude native",
          enabled: true,
          orderedProfileIds: [],
          balanceRemainingQuota: false,
          revision: 1,
        });
      }
      return result;
    };
  });
  const dialog = await openNativeModelDialog(page);
  await chooseRoute(dialog, "claude-native");
  const mode = dialog.getByRole("radiogroup", {
    name: "Execution mode",
    exact: true,
  });
  await expect(mode.locator('input[value="native"]')).toHaveCount(1);
  await expect(mode.locator("input:checked")).toHaveCount(0);
  await dialog.getByText("Open a terminal instead", { exact: true }).click();
  await expect(
    dialog.getByRole("button", {
      name: "Open native account terminal",
      exact: true,
    }),
  ).toBeVisible();
  await chooseMode(dialog, "native");
  await expect(dialog.locator(".router-launch-selection")).toContainText(
    "CLI account",
  );

  const model = runField(dialog, "Model");
  expect(await model.evaluate((element) => element.tagName)).toBe("INPUT");
  await model.fill("claude-exact-native-model");

  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await dialog.getByRole("button", { name: "Start run", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).nativeModelStartCalls))
    .toEqual([
      expect.objectContaining({
        routerId: "claude-native",
        executionMode: "native",
        model: "claude-exact-native-model",
        reasoningEffort: null,
      }),
    ]);
});

async function supervisedNativePaneFixture(
  page: Page,
  permission: boolean | "text" = false,
) {
  const { newProject, newSession, newCliAgentTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const tab = newCliAgentTab("native-run", "Native task");
  project.workspaces[0].tabs = [tab];
  project.workspaces[0].activeTabId = tab.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(
    ({ permission }) => {
      const desktop = window as any;
      const original = desktop.__TAURI_INTERNALS__.invoke;
      const run = {
        id: "native-run",
        routerId: "native-pool",
        cwd: "/project",
        shellProfileId: "local:bash",
        title: "Native task",
        executionMode: "native",
        state: permission ? "running" : "recovery_required",
        model: "claude-exact-native-model",
        reasoningEffort: null,
        pinnedProfileId: "original",
        allowedProfileIds: ["original"],
        activeProfileId: "original",
        generation: 4,
        revision: 7,
        inputs: [{ id: "assignment", text: "Original assignment" }],
        attempts: [],
        turns: [],
        output: "",
        statusMessage: "Native effects require review",
        attemptedProfileIds: ["original"],
      };
      const snapshot = {
        revision: 1,
        profiles: [
          {
            id: "original",
            cli: "claude",
            label: "Original account",
            enabled: true,
            revision: 2,
            authState: "ready",
            storageMode: "cli_managed",
          },
        ],
        routers: [
          {
            id: "native-pool",
            cli: "claude",
            label: "Native pool",
            enabled: true,
            orderedProfileIds: ["original"],
            balanceRemainingQuota: false,
            revision: 1,
          },
        ],
        capabilities: [
          {
            cli: "claude",
            name: "Claude Code",
            managedTurns: false,
            nativeTurns: true,
            nativeAccountTerminal: true,
          },
        ],
        quota: [],
        runs: [run],
      };
      let pending = permission
        ? [
            {
              id: "native-approval",
              runId: "native-run",
              generation: 4,
              tool: "shell",
              input: { command: "echo explicit-review" },
              choices: [
                { id: "deny", label: "Deny operation", allow: false },
                { id: "once", label: "Allow once", allow: true },
              ],
              ...(permission === "text"
                ? {
                    textInput: {
                      label: "Native patch notes",
                      initial: "Existing notes",
                      multiline: true,
                    },
                    choices: [
                      { id: "cancel", label: "Cancel", allow: false },
                      { id: "submit-text", label: "Submit", allow: true },
                    ],
                  }
                : {}),
            },
          ]
        : [];
      desktop.supervisedNativeCalls = [];
      desktop.nativePermissionReads = 0;
      desktop.__TAURI_INTERNALS__.invoke = async (
        command: string,
        args: any,
      ) => {
        if (command === "cli_router_snapshot") return structuredClone(snapshot);
        if (command === "cli_native_permissions") {
          desktop.nativePermissionReads++;
          return structuredClone(pending);
        }
        if (command === "cli_native_permission_reply") {
          desktop.supervisedNativeCalls.push({
            command,
            request: args.request,
          });
          pending = [];
          return null;
        }
        if (command === "cli_run_send") {
          desktop.supervisedNativeCalls.push({
            command,
            request: args.request,
          });
          throw new Error(
            "RecoveryRequired: native checkpoint is uncertain; review the original account session",
          );
        }
        if (command === "cli_run_open_native_recovery") {
          desktop.supervisedNativeCalls.push({
            command,
            runId: args.runId,
            expectedRevision: args.expectedRevision,
          });
          return structuredClone(snapshot);
        }
        if (command === "cli_run_acknowledge_coding_completion") {
          desktop.supervisedNativeCalls.push({
            command,
            request: args.request,
          });
          run.state = "idle";
          run.revision++;
          snapshot.revision++;
          return structuredClone(snapshot);
        }
        if (command === "start_terminal" || command === "cli_run_start") {
          desktop.supervisedNativeCalls.push({ command });
          throw new Error("Unexpected implicit native launch");
        }
        return original(command, args);
      };
    },
    { permission },
  );
  await page.goto("/");
  return page.locator(".cli-agent-pane");
}

test("native permission stays pending until an explicit generation-bound decision", async ({
  page,
}) => {
  await supervisedNativePaneFixture(page, true);
  const review = page.getByRole("dialog", {
    name: "Review native CLI permission",
    exact: true,
  });
  await expect(review).toContainText("echo explicit-review");
  await expect(
    review.getByRole("button", { name: "Deny operation", exact: true }),
  ).toBeFocused();
  expect(
    await page.evaluate(() => (window as any).supervisedNativeCalls),
  ).toEqual([]);
  await review
    .getByRole("button", { name: "Deny operation", exact: true })
    .click();
  await expect(review).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => (window as any).supervisedNativeCalls))
    .toEqual([
      {
        command: "cli_native_permission_reply",
        request: {
          approvalId: "native-approval",
          runId: "native-run",
          generation: 4,
          choiceId: "deny",
        },
      },
    ]);
});

test("uncertain native recovery offers the original account terminal after checkpoint refusal", async ({
  page,
}) => {
  const pane = await supervisedNativePaneFixture(page);
  await expect(
    pane.getByRole("button", { name: "Continue saved task", exact: true }),
  ).toHaveCount(0);
  await expect(pane).toContainText("original account");
  expect(
    await page.evaluate(() => (window as any).supervisedNativeCalls),
  ).toEqual([]);
  await pane
    .getByRole("button", { name: "Resume checkpointed task", exact: true })
    .click();
  await expect(pane).toContainText("native checkpoint is uncertain");
  await pane
    .getByRole("button", {
      name: "Review in native account terminal",
      exact: true,
    })
    .click();
  const calls = await page.evaluate(
    () => (window as any).supervisedNativeCalls,
  );
  expect(calls).toEqual([
    {
      command: "cli_run_send",
      request: expect.objectContaining({
        runId: "native-run",
        expectedRevision: 7,
        handoff: true,
        text: "",
      }),
    },
    {
      command: "cli_run_open_native_recovery",
      runId: "native-run",
      expectedRevision: 7,
    },
  ]);
  expect(
    calls.filter((call: any) =>
      ["cli_run_start", "start_terminal"].includes(call.command),
    ),
  ).toEqual([]);
});

test("native recorded-completion acknowledgement returns to idle without launching or replaying", async ({
  page,
}) => {
  const pane = await supervisedNativePaneFixture(page);
  await pane
    .getByRole("button", {
      name: "Acknowledge recorded completion",
      exact: true,
    })
    .click();
  await expect(
    pane.getByRole("textbox", { name: "Message", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(() => (window as any).supervisedNativeCalls),
  ).toEqual([
    {
      command: "cli_run_acknowledge_coding_completion",
      request: {
        runId: "native-run",
        expectedRunRevision: 7,
        requestId: expect.stringMatching(
          /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
        ),
      },
    },
  ]);
});

test("native recovery event opens the original account with its run revision and no prompt", async ({
  page,
}) => {
  const { newProject, newSession, newBrowserTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const browser = newBrowserTab();
  project.workspaces[0].tabs = [browser];
  project.workspaces[0].activeTabId = browser.id;
  await mockDesktop(page, false, {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  });
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    desktop.nativeRecoveryRequests = [];
    desktop.nativeRecoverySubmissions = [];
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot")
        return {
          revision: 1,
          profiles: [
            {
              id: "original",
              cli: "claude",
              label: "Original account",
              enabled: true,
            },
          ],
          routers: [],
          capabilities: [],
          quota: [],
          runs: [],
        };
      if (command === "start_terminal")
        desktop.nativeRecoveryRequests.push(args.request);
      if (["cli_run_send", "cli_run_start", "write_terminal"].includes(command))
        desktop.nativeRecoverySubmissions.push({ command, ...args });
      return original(command, args);
    };
  });
  await page.goto("/");
  await expect(page.getByRole("button", { name: /^New tab/ })).toBeVisible();
  await page.evaluate(async () => {
    await (window as any).__nativeTest.emitEvent("cli-router-open-profile", {
      profileId: "original",
      cli: "claude",
      cwd: "/project",
      shellProfileId: "local:bash",
      nativeRunId: "native-run",
      nativeRunRevision: 7,
    });
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).nativeRecoveryRequests))
    .toEqual([
      expect.objectContaining({
        cwd: "/project",
        profileId: "local:bash",
        cliLaunch: "claude",
        routerProfileId: "original",
        routerId: null,
        gatewayRunId: null,
        gatewayRunRevision: null,
        nativeRecovery: { runId: "native-run", revision: 7 },
      }),
    ]);
  expect(
    await page.evaluate(() => (window as any).nativeRecoverySubmissions),
  ).toEqual([]);
});

test("native editor input preserves manual edits across permission polling", async ({
  page,
}) => {
  await supervisedNativePaneFixture(page, "text");
  const dialog = page.getByRole("dialog", {
    name: "Native CLI interaction",
    exact: true,
  });
  const input = dialog.getByRole("textbox", {
    name: "Native patch notes",
    exact: true,
  });
  await expect(input).toHaveValue("Existing notes");
  await input.fill(
    "Keep completed changes and review the uncertain tool result.",
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).nativePermissionReads))
    .toBeGreaterThan(2);
  await expect(input).toHaveValue(
    "Keep completed changes and review the uncertain tool result.",
  );
  expect(
    await page.evaluate(() => (window as any).supervisedNativeCalls),
  ).toEqual([]);
  await page.screenshot({ path: "test-results/router-native-editor.png" });
  await dialog.getByRole("button", { name: "Submit", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as any).supervisedNativeCalls),
  ).toEqual([
    {
      command: "cli_native_permission_reply",
      request: {
        approvalId: "native-approval",
        runId: "native-run",
        generation: 4,
        choiceId: "submit-text",
        text: "Keep completed changes and review the uncertain tool result.",
      },
    },
  ]);
});

// These fixtures expose an owned, settled Pi history through IPC only. No
// native process, provider, authentication flow or permission reply is invoked.
async function piNativeHandoffFixture(page: Page) {
  const { newProject, newSession, newCliAgentTab } =
    await import("../../src/model");
  const project = newProject("/project", "local:bash");
  const tab = newCliAgentTab("pi-handoff-run", "Pi saved task");
  project.workspaces[0].tabs = [tab];
  project.workspaces[0].activeTabId = tab.id;
  await mockDesktop(
    page,
    false,
    {
      ...newSession(),
      projects: [project],
      activeProjectId: project.id,
    },
    undefined,
    undefined,
    undefined,
    {
      installed: [{ cli: "pi", name: "Pi", command: "pi" }],
    },
  );
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const profiles = [
      { id: "original", label: "Original Pi account", revision: 2 },
      { id: "later", label: "Later Pi account", revision: 3 },
      {
        id: "backup",
        label: "Backup Pi account",
        revision: 4,
        authState: "unverified",
      },
      { id: "api", label: "API account", revision: 1, storageMode: "api_key" },
      {
        id: "not-granted",
        label: "Account without history access",
        revision: 1,
      },
      { id: "outsider", label: "Account outside router", revision: 1 },
      {
        id: "disabled",
        label: "Disabled account",
        revision: 1,
        enabled: false,
      },
    ].map((profile) => ({
      cli: "pi",
      enabled: true,
      authState: "ready",
      storageMode: "cli_managed",
      ...profile,
    }));
    const run = {
      id: "pi-handoff-run",
      routerId: "pi-pool",
      cwd: "/project",
      shellProfileId: "local:bash",
      title: "Pi saved task",
      executionMode: "native",
      state: "completed",
      model: "openai/gpt-4.1",
      reasoningEffort: null,
      pinnedProfileId: "original",
      activeProfileId: "original" as string | null,
      allowedProfileIds: [
        "original",
        "later",
        "backup",
        "api",
        "outsider",
        "disabled",
      ],
      generation: 4,
      revision: 7,
      inputs: [{ id: "pi-input", text: "Original saved assignment" }],
      attempts: [],
      turns: [
        {
          inputId: "pi-input",
          attemptId: "pi-attempt",
          profileId: "original",
          generation: 4,
          state: "completed",
          text: "Saved native result",
        },
      ],
      output: "Saved native result",
      statusMessage: "Settled native checkpoint",
      attemptedProfileIds: ["original"],
    };
    const snapshot = {
      revision: 1,
      profiles,
      runs: [run],
      quota: [],
      routers: [
        {
          id: "pi-pool",
          cli: "pi",
          label: "Pi pool",
          enabled: true,
          orderedProfileIds: [
            "original",
            "later",
            "backup",
            "api",
            "not-granted",
            "disabled",
          ],
          balanceRemainingQuota: false,
          revision: 1,
        },
      ],
      capabilities: [
        {
          cli: "pi",
          name: "Pi",
          managedTurns: false,
          nativeTurns: true,
          nativeAccountTerminal: true,
        },
      ],
    };
    desktop.piHandoffSnapshot = snapshot;
    desktop.piHandoffCalls = [];
    desktop.piHandoffEffects = [];
    desktop.changePiHandoffIdentity = async (kind: "profile" | "run") => {
      if (kind === "profile")
        profiles.find((profile) => profile.id === "later")!.revision++;
      else run.revision++;
      snapshot.revision++;
      await desktop.__nativeTest.emitEvent("cli-router-changed", {
        revision: snapshot.revision,
      });
    };
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_native_permissions") return [];
      if (command === "cli_native_handoff_preview") {
        desktop.piHandoffCalls.push({
          command,
          request: structuredClone(args.request),
        });
        const destination = profiles.find(
          (profile) => profile.id === args.request.destinationProfileId,
        );
        if (
          !destination ||
          args.request.expectedRevision !== run.revision ||
          args.request.destinationProfileRevision !== destination.revision
        )
          throw new Error("Stale native handoff preview");
        return {
          runId: run.id,
          runRevision: run.revision,
          sourceProfileId: run.activeProfileId,
          sourceProfileRevision: 2,
          sourceLabel: "Original Pi account",
          destinationProfileId: destination.id,
          destinationProfileRevision: destination.revision,
          destinationLabel: destination.label,
          generation: run.generation,
          inputId: "pi-input",
          model: run.model,
          version: "1.0.1",
          historyDigest: "a".repeat(64),
          historyBytes: 24576,
        };
      }
      if (command === "cli_native_handoff_apply") {
        desktop.piHandoffCalls.push({
          command,
          request: structuredClone(args.request),
        });
        const review = args.request.review;
        const destination = profiles.find(
          (profile) => profile.id === review.destinationProfileId,
        );
        if (
          !destination ||
          review.runRevision !== run.revision ||
          review.sourceProfileRevision !== profiles[0].revision ||
          review.destinationProfileRevision !== destination.revision ||
          review.historyDigest !== "a".repeat(64)
        )
          throw new Error("Stale reviewed native identity or history");
        run.pinnedProfileId = destination.id;
        run.activeProfileId = null;
        run.state = "idle";
        run.revision++;
        snapshot.revision++;
        await desktop.__nativeTest.emitEvent("cli-router-changed", {
          revision: snapshot.revision,
        });
        return structuredClone(snapshot);
      }
      if (
        command.startsWith("cli_run_") ||
        [
          "start_terminal",
          "start_profile_terminal",
          "cli_native_permission_reply",
          "cli_run_update_data_grant",
        ].includes(command)
      ) {
        desktop.piHandoffEffects.push({ command, ...args });
        throw new Error(
          "History transfer must not launch, send, grant access or answer permissions",
        );
      }
      return original(command, args);
    };
  });
  await page.goto("/");
  return page.getByRole("region", { name: "Pi saved task", exact: true });
}

test("Pi native handoff requires reviewed history consent and preserves the draft without a turn", async ({
  page,
}) => {
  const pane = await piNativeHandoffFixture(page);
  const message = pane.getByRole("textbox", { name: "Message", exact: true });
  await message.fill("Draft reserved for the next explicit Pi turn");
  await pane
    .getByRole("button", { name: "Transfer Pi native history", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Transfer Pi native history",
    exact: true,
  });
  const apply = review.getByRole("button", {
    name: "Transfer native history",
    exact: true,
  });
  const destination = review.getByRole("combobox", {
    name: "Destination account",
    exact: true,
  });
  await expect(destination.locator("option")).toHaveText([
    "Choose an approved account",
    "Later Pi account",
    "Backup Pi account",
  ]);
  await expect(apply).toBeDisabled();
  await destination.selectOption("later");
  await expect(apply).toBeDisabled();
  await review
    .getByRole("button", { name: "Review transfer", exact: true })
    .click();
  const consent = review.getByRole("checkbox", {
    name: /I approve copying this native history/,
  });
  await expect(consent).not.toBeChecked();
  await expect(review).toContainText("Original Pi account");
  await expect(review).toContainText("Later Pi account");
  await expect(review).toContainText("openai/gpt-4.1");
  await expect(apply).toBeDisabled();
  expect(await page.evaluate(() => (window as any).piHandoffCalls)).toEqual([
    {
      command: "cli_native_handoff_preview",
      request: {
        runId: "pi-handoff-run",
        expectedRevision: 7,
        destinationProfileId: "later",
        destinationProfileRevision: 3,
      },
    },
  ]);
  await consent.check();
  await apply.click();
  await expect(review).toHaveCount(0);
  await expect(
    pane.getByRole("combobox", {
      name: "Account for next attempt",
      exact: true,
    }),
  ).toHaveValue("later");
  await expect(message).toHaveValue(
    "Draft reserved for the next explicit Pi turn",
  );
  await expect(pane.getByRole("status")).toContainText(
    "Native history transferred to Later Pi account",
  );
  const calls = await page.evaluate(() => (window as any).piHandoffCalls);
  expect(calls).toHaveLength(2);
  expect(calls[1]).toEqual({
    command: "cli_native_handoff_apply",
    request: {
      requestId: expect.stringMatching(
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
      ),
      review: {
        runId: "pi-handoff-run",
        runRevision: 7,
        sourceProfileId: "original",
        sourceProfileRevision: 2,
        sourceLabel: "Original Pi account",
        destinationProfileId: "later",
        destinationProfileRevision: 3,
        destinationLabel: "Later Pi account",
        generation: 4,
        inputId: "pi-input",
        model: "openai/gpt-4.1",
        version: "1.0.1",
        historyDigest: "a".repeat(64),
        historyBytes: 24576,
      },
    },
  });
  expect(
    await page.evaluate(() => (window as any).piHandoffSnapshot.runs[0]),
  ).toMatchObject({
    revision: 8,
    pinnedProfileId: "later",
    activeProfileId: null,
    state: "idle",
    generation: 4,
    inputs: [{ id: "pi-input", text: "Original saved assignment" }],
  });
  expect(await page.evaluate(() => (window as any).piHandoffEffects)).toEqual(
    [],
  );
});

test("Pi native handoff invalidates changed identities and nested cancel preserves both drafts", async ({
  page,
}) => {
  const pane = await piNativeHandoffFixture(page);
  const paneDraft = pane.getByRole("textbox", { name: "Message", exact: true });
  await paneDraft.fill("Keep the workspace draft");
  // Open the outer run dialog without navigating: a reload would reset the
  // retained RunView draft and would miss the nested-modal regression.
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "Agents", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Agents", exact: true })
    .getByRole("button", { name: "Open router", exact: true })
    .click();
  const outer = page.locator("dialog.router-launch-dialog");
  await outer.getByRole("button", { name: /^History/ }).click();
  if (await outer.getByText("All runs", { exact: true }).isVisible())
    await outer.getByText("All runs", { exact: true }).click();
  await outer
    .getByRole("combobox", { name: "Run history", exact: true })
    .selectOption("pi-handoff-run");
  const outerDraft = outer.getByRole("textbox", {
    name: "Message",
    exact: true,
  });
  await outerDraft.fill("Keep the router dialog draft");
  await outer
    .getByRole("button", { name: "Transfer Pi native history", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Transfer Pi native history",
    exact: true,
  });
  const destination = review.getByRole("combobox", {
    name: "Destination account",
    exact: true,
  });
  const consent = review.getByRole("checkbox", {
    name: /I approve copying this native history/,
  });
  const apply = review.getByRole("button", {
    name: "Transfer native history",
    exact: true,
  });
  const preview = review.getByRole("button", {
    name: "Review transfer",
    exact: true,
  });
  await destination.selectOption("later");
  await preview.click();
  await consent.check();
  await page.evaluate(() => (window as any).changePiHandoffIdentity("profile"));
  await expect(consent).toHaveCount(0);
  await expect(apply).toBeDisabled();
  await review
    .locator("form")
    .evaluate((form) =>
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      ),
    );
  await preview.click();
  await expect(consent).not.toBeChecked();
  await consent.check();
  await page.evaluate(() => (window as any).changePiHandoffIdentity("run"));
  await expect(consent).toHaveCount(0);
  await expect(apply).toBeDisabled();
  await preview.click();
  await expect(consent).not.toBeChecked();
  await consent.check();
  await destination.selectOption("backup");
  await expect(consent).toHaveCount(0);
  await expect(apply).toBeDisabled();
  await review.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(review).toHaveCount(0);
  await expect(outer).toBeVisible();
  await expect(outerDraft).toHaveValue("Keep the router dialog draft");
  await outer
    .getByRole("button", { name: "Transfer Pi native history", exact: true })
    .click();
  await destination.selectOption("backup");
  await preview.click();
  await consent.check();
  await page.keyboard.press("Escape");
  await expect(review).toHaveCount(0);
  await expect(outer).toBeVisible();
  await expect(
    outer.getByRole("combobox", { name: "Run history", exact: true }),
  ).toHaveValue("pi-handoff-run");
  await expect(outerDraft).toHaveValue("Keep the router dialog draft");
  await expect(paneDraft).toHaveValue("Keep the workspace draft");
  const calls = await page.evaluate(() => (window as any).piHandoffCalls);
  expect(
    calls.filter((call: any) => call.command === "cli_native_handoff_apply"),
  ).toEqual([]);
  expect(
    calls
      .filter((call: any) => call.command === "cli_native_handoff_preview")
      .map((call: any) => call.request),
  ).toEqual([
    {
      runId: "pi-handoff-run",
      expectedRevision: 7,
      destinationProfileId: "later",
      destinationProfileRevision: 3,
    },
    {
      runId: "pi-handoff-run",
      expectedRevision: 7,
      destinationProfileId: "later",
      destinationProfileRevision: 4,
    },
    {
      runId: "pi-handoff-run",
      expectedRevision: 8,
      destinationProfileId: "later",
      destinationProfileRevision: 4,
    },
    {
      runId: "pi-handoff-run",
      expectedRevision: 8,
      destinationProfileId: "backup",
      destinationProfileRevision: 4,
    },
  ]);
  expect(await page.evaluate(() => (window as any).piHandoffEffects)).toEqual(
    [],
  );
});

test("one connection save binds a key to the returned canonical endpoint revision", async ({
  page,
}) => {
  await gatewaySettingsFixture(page);
  await page
    .getByLabel("API endpoint", { exact: true })
    .fill("https://api.example.test/");
  await page
    .getByLabel("API key", { exact: true })
    .fill("reviewed-destination-key");
  await page
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Subscription account", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Manage Subscription account", exact: true })
    .click();
  await expect(page.getByLabel("API endpoint", { exact: true })).toHaveValue(
    "https://api.example.test",
  );
  await expect(page.getByLabel("API key", { exact: true })).toHaveValue("");
  const calls = await page.evaluate(() => (window as any).gatewaySettingsCalls);
  expect(calls.map((call: any) => call.command)).toEqual([
    "cli_router_mutate",
    "cli_profile_set_api_key",
  ]);
  expect(calls[1]).toEqual(
    expect.objectContaining({
      profileId: "subscription",
      expectedRevision: 2,
      key: "reviewed-destination-key",
    }),
  );
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await page.screenshot({ path: "test-results/router-ux-api.png" });
});

test("a changed returned gateway destination blocks key binding and preserves its draft", async ({
  page,
}) => {
  await gatewaySettingsFixture(page);
  await page.evaluate(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      const result = await original(command, args);
      if (
        command === "cli_router_mutate" &&
        args.request.action.type === "configure_gateway_account"
      )
        result.profiles[0].gatewayProvider.baseUrl =
          "https://unreviewed.example.test";
      return result;
    };
  });
  await page
    .getByLabel("API endpoint", { exact: true })
    .fill("https://reviewed.example.test");
  await page
    .getByLabel("API key", { exact: true })
    .fill("keep-unbound-key-draft");
  await page
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(
    page.getByText(
      "The account destination changed. Review it before saving a key.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(page.getByLabel("API key", { exact: true })).toHaveValue(
    "keep-unbound-key-draft",
  );
  expect(
    await page.evaluate(() =>
      (window as any).gatewaySettingsCalls.map((call: any) => call.command),
    ),
  ).toEqual(["cli_router_mutate"]);
});

test("a managed API key stays direct until the user supplies a gateway endpoint", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript(() => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const profile = {
      id: "direct",
      cli: "claude",
      label: "Direct account",
      enabled: true,
      revision: 1,
      authState: "disconnected",
      storageMode: "cli_managed",
      gatewayProvider: null,
    };
    const snapshot = {
      revision: 1,
      profiles: [profile],
      routers: [],
      quota: [],
      runs: [],
      capabilities: [
        {
          cli: "claude",
          name: "Claude Code",
          canCreateProfile: true,
          canVerifyLogin: true,
          profileTerminal: true,
          managedTurns: true,
          apiKeyLabel: "Anthropic API key",
          gatewayTerminal: true,
          gatewayProtocol: "anthropic",
        },
      ],
    };
    desktop.directApiCalls = [];
    desktop.__TAURI_INTERNALS__.invoke = async (command: string, args: any) => {
      if (command === "cli_router_snapshot") return structuredClone(snapshot);
      if (command === "cli_profile_native_report") return null;
      if (command === "cli_profile_set_api_key") {
        if (args.expectedRevision !== profile.revision)
          throw new Error("Stale direct key review");
        desktop.directApiCalls.push({ command, ...args });
        profile.revision++;
        profile.authState = "ready";
        profile.storageMode = "api_key";
        snapshot.revision++;
        return structuredClone(snapshot);
      }
      if (
        [
          "cli_router_mutate",
          "cli_profile_open_terminal",
          "cli_run_start",
          "start_terminal",
        ].includes(command)
      ) {
        desktop.directApiCalls.push({ command });
        throw new Error(
          "A direct key must not configure a gateway or launch work",
        );
      }
      return original(command, args);
    };
  });
  await page.goto("/?window=settings&page=agent-control");
  await page.getByRole("tab", { name: "Router", exact: true }).click();
  await chooseCli(page, "claude");
  await page
    .getByRole("button", { name: "Manage Direct account", exact: true })
    .click();
  await page.getByRole("button", { name: "API key", exact: true }).click();
  const endpoint = page.getByLabel("API endpoint", { exact: true });
  await expect(endpoint).toHaveValue("");
  await page
    .getByLabel("Anthropic API key", { exact: true })
    .fill("direct-provider-key");
  await page
    .getByRole("button", { name: "Save connection", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Direct account", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Manage Direct account", exact: true })
    .click();
  expect(await page.evaluate(() => (window as any).directApiCalls)).toEqual([
    {
      command: "cli_profile_set_api_key",
      profileId: "direct",
      expectedRevision: 1,
      key: "direct-provider-key",
    },
  ]);
  await endpoint.fill("https://openai.example.test");
  await page
    .getByRole("dialog", { name: "Direct account", exact: true })
    .getByText("Advanced", { exact: true })
    .click();
  const protocol = page.getByRole("combobox", {
    name: "API protocol",
    exact: true,
  });
  await protocol.focus();
  await protocol.press("Space");
  await expect(
    page
      .getByRole("dialog", { name: "Direct account", exact: true })
      .getByRole("listbox"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(protocol).toHaveAttribute("aria-expanded", "false");
  await expect(
    page.getByRole("dialog", { name: "Direct account", exact: true }),
  ).toBeVisible();
  await expect(endpoint).toHaveValue("https://openai.example.test");
  await protocol.press("Space");
  await protocol.press("ArrowDown");
  await protocol.press("Enter");
  await expect(protocol).toHaveText("OpenAI Chat Completions");
  expect(
    await page.evaluate(() =>
      (window as any).directApiCalls.map((call: any) => call.command),
    ),
  ).toEqual(["cli_profile_set_api_key"]);
  await page.screenshot({ path: "test-results/router-style-api-advanced.png" });
  await expect(page.getByLabel("API key", { exact: true })).toBeVisible();
  await expect(
    page.getByLabel("Anthropic API key", { exact: true }),
  ).toHaveCount(0);
});
