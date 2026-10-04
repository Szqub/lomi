import { expect, test, type Page, type Locator } from "@playwright/test";
import { mockDesktop } from "./desktop";
import { cliNames } from "../../src/cli-agents";

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

test("router tab lists every CLI and saves account order with revision fencing", async ({
  page,
}) => {
  await mockDesktop(page, false, null);
  await page.addInitScript((names) => {
    const desktop = window as any;
    const original = desktop.__TAURI_INTERNALS__.invoke;
    const snapshot = {
      revision: 1,
      capabilities: Object.entries(names).map(([cli]) => ({
        cli,
        canCreateProfile: ["codex", "claude", "gemini", "pi", "goose"].includes(
          cli,
        ),
        canVerifyLogin: cli === "codex" || cli === "claude",
        canStart: ["codex", "claude", "gemini", "pi", "goose"].includes(cli),
        profileTerminal: ["codex", "claude", "gemini", "pi"].includes(cli),
        managedTurns: ["claude", "gemini", "pi", "goose"].includes(cli),
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
      profiles: ["first", "second"].map((id) => ({
        id,
        cli: "codex",
        label: id === "first" ? "Personal" : "Work",
        enabled: true,
        revision: 1,
        authState: "ready",
        storageMode: "cli_managed",
      })),
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
  await expect(page.locator("#router-cli option")).toHaveCount(32);
  await page.getByLabel("CLI agent", { exact: true }).selectOption("aider");
  await expect(
    page.getByText("Separate accounts are not supported for this CLI yet.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Add account", exact: true }),
  ).toHaveCount(0);
  await page.getByLabel("CLI agent", { exact: true }).selectOption("codex");
  await page.getByRole("button", { name: "Move Work up", exact: true }).click();
  await expect(
    page
      .getByRole("list", { name: "Account order for Daily router" })
      .getByRole("listitem")
      .first(),
  ).toContainText("Work");
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).routerCalls[0]?.expectedRevision),
    )
    .toBe(1);
  await page
    .getByRole("checkbox", {
      name: "Balance remaining quota for Daily router",
      exact: true,
    })
    .check();
  await expect(
    page.getByText(
      "Fresh quota reports are unavailable. Runs use account order.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).routerCalls[1]?.expectedRevision),
    )
    .toBe(2);
  await expect(
    page.getByText(
      "Separate account terminals and quota are available. Managed runs are unavailable.",
      { exact: true },
    ),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Remove", exact: true })
    .first()
    .click();
  await expect(
    page.getByText("API key removal is pending. Retry removal.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByText("pending remove · Quota unknown", { exact: true }),
  ).toBeVisible();
  const pending = page.locator(".router-profile-row").filter({
    has: page.getByRole("checkbox", { name: "Enable Personal", exact: true }),
  });
  await expect(
    pending.getByRole("checkbox", { name: "Enable Personal", exact: true }),
  ).toBeDisabled();
  await expect(
    pending.getByRole("checkbox", { name: "Enable Personal", exact: true }),
  ).not.toBeChecked();
  await expect(
    pending.getByRole("button", { name: "Open CLI to sign in", exact: true }),
  ).toBeDisabled();
  await expect(
    pending.getByRole("button", { name: "Verify", exact: true }),
  ).toBeDisabled();
  await expect(
    pending.getByRole("button", { name: "Remove", exact: true }),
  ).toBeEnabled();
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
  const dialog = page.getByRole("dialog", {
    name: "Routed CLI runs",
    exact: true,
  });
  await dialog
    .getByRole("combobox", { name: "Router", exact: true })
    .selectOption("pool");
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toBeDisabled();
  await expect(
    dialog.getByText(
      "Text-only CLI run. Project files and tools are unavailable. Every turn starts a fresh conversation.",
      { exact: true },
    ),
  ).toBeVisible();
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
  ).toHaveCount(1);
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
  const dialog = page.getByRole("dialog", {
    name: "Routed CLI runs",
    exact: true,
  });
  await dialog
    .getByRole("combobox", { name: "Router", exact: true })
    .selectOption("codex-pool");
  await expect(dialog.getByLabel("Model", { exact: true })).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Start run", exact: true }),
  ).toHaveCount(0);
  await expect(
    dialog.getByText(
      "Account order chooses a new terminal only. Running CLI sessions keep their accounts.",
      { exact: false },
    ),
  ).toBeVisible();
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
  await page.getByLabel("CLI agent", { exact: true }).selectOption("codex");
  await expect(
    page.getByText("ready · 40% remaining", { exact: true }),
  ).toHaveCount(2);
  await expect(
    page.getByText(
      "Fresh quota reports are unavailable. Runs use account order.",
      { exact: true },
    ),
  ).toHaveCount(0);
  const reads = await page.evaluate(() => (window as any).quotaSnapshotReads);
  await page.clock.fastForward(2001);
  await expect(
    page.getByText("ready · Quota stale", { exact: true }),
  ).toHaveCount(2);
  await expect(
    page.getByText(
      "Fresh quota reports are unavailable. Runs use account order.",
      { exact: true },
    ),
  ).toBeVisible();
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
          cli: "goose",
          name: "Goose",
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
          cli: "goose",
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
  await page.getByLabel("CLI agent", { exact: true }).selectOption("goose");
  const login = page.getByRole("button", { name: "Open CLI to sign in" });
  await expect(login).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Verify", exact: true }),
  ).toBeDisabled();
  const key = page.getByLabel("OpenAI API key", { exact: true });
  await key.fill("fixture-api-key-for-source-regression");
  await page.getByRole("button", { name: "Save API key", exact: true }).click();
  await expect(key).toHaveValue("");
  await expect(
    page.getByRole("button", { name: "Verify", exact: true }),
  ).toBeEnabled();
  await expect(login).toBeDisabled();
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
          cli: "goose",
          name: "Goose",
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
      routers: ["codex", "openclaw", "goose"].map((cli) => ({
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
  return page.getByRole("dialog", { name: "Routed CLI runs", exact: true });
}

test("native model catalogs require explicit choices and fence efforts across CLI changes", async ({
  page,
}) => {
  await nativeModelFixture(page);
  const dialog = await openNativeModelDialog(page);
  const router = dialog.getByRole("combobox", { name: "Router", exact: true });
  const model = runField(dialog, "Model");
  const effort = runField(dialog, "Reasoning effort");
  const start = dialog.getByRole("button", { name: "Start run", exact: true });
  await router.selectOption("codex-pool");
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
  await model.selectOption("gpt-6-luna");
  await expect(effort).toHaveValue("");
  await expect(effort.locator("option")).toHaveText([
    "Choose reasoning effort",
    "low",
  ]);
  await expect(start).toBeDisabled();
  await effort.selectOption("low");
  await router.selectOption("openclaw-pool");
  await expect(model).toHaveValue("");
  await expect(model.locator("option")).toHaveText([
    "Choose a model",
    "gpt-4.1",
  ]);
  await expect(effort).toHaveCount(0);
  await expect(start).toBeDisabled();
  await model.selectOption("gpt-4.1");
  await expect(start).toBeEnabled();
  await router.selectOption("codex-pool");
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
  await router.selectOption("goose-pool");
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
  await dialog
    .getByRole("combobox", { name: "Router", exact: true })
    .selectOption("codex-pool");
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
  await page.getByLabel("CLI agent", { exact: true }).selectOption("codex");
  await expect(
    page.getByText(
      "Managed text turns require a verified subscription account and a current native quota report.",
      { exact: false },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open CLI to sign in", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Verify", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Save API key", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.locator('.router-profile-row input[type="password"]'),
  ).toHaveCount(0);
  await expect(
    page.getByText("Save your own API key", { exact: false }),
  ).toHaveCount(0);
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
  await dialog
    .getByRole("combobox", { name: "Router", exact: true })
    .selectOption("codex-pool");
  const mode = dialog.getByRole("combobox", {
    name: "Execution mode",
    exact: true,
  });
  const model = runField(dialog, "Model");
  const effort = runField(dialog, "Reasoning effort");
  const start = dialog.getByRole("button", { name: "Start run", exact: true });
  await expect(mode).toHaveValue("");
  await mode.selectOption("coding");
  await expect(model).toHaveValue("");
  await expect(start).toBeDisabled();
  await expect(dialog).toContainText(
    "Every write or patch requires your explicit approval",
  );
  await model.selectOption("gpt-6-sol");
  await effort.selectOption("high");
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await page.evaluate(() => (window as any).setCodingAvailable(false));
  await expect(mode.locator('option[value="coding"]')).toHaveCount(0);
  await expect(mode).toHaveValue("text");
  await expect(model).toHaveValue("");
  await expect(effort).toHaveValue("");
  await expect(start).toBeDisabled();
  await start.evaluate((button) =>
    button
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(
    await page.evaluate(() => (window as any).nativeModelStartCalls),
  ).toEqual([]);
  await page.evaluate(() => (window as any).setCodingAvailable(true));
  await expect(mode).toHaveValue("");
  await mode.selectOption("coding");
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
  await dialog
    .getByRole("combobox", { name: "Router", exact: true })
    .selectOption("openclaw-pool");
  await expect(mode).toHaveValue("text");
  await expect(mode.locator("option")).toHaveText([
    "Choose an execution mode",
    "Text",
  ]);
  await expect(model).toHaveValue("");
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
  await dialog
    .getByRole("combobox", { name: "Router", exact: true })
    .selectOption("codex-pool");
  await dialog
    .getByRole("combobox", { name: "Execution mode", exact: true })
    .selectOption("gateway");
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
  await page.getByLabel("CLI agent", { exact: true }).selectOption("codex");
}

test("gateway endpoint canonicalization enables key save and endpoint changes discard old key drafts", async ({
  page,
}) => {
  await gatewaySettingsFixture(page);
  const endpoint = page.getByLabel("Gateway API endpoint", {
    exact: true,
  });
  const saveEndpoint = page.getByRole("button", {
    name: "Save gateway endpoint",
    exact: true,
  });
  await expect(page.getByLabel("Gateway API key", { exact: true })).toHaveCount(
    0,
  );
  await endpoint.fill("https://api.example.test/");
  await saveEndpoint.click();
  // The native snapshot owns the canonical endpoint; a stale slash-bearing
  // input draft must not prevent saving a key for that exact destination.
  await expect(endpoint).toHaveValue("https://api.example.test");
  const key = page.getByLabel("Gateway API key", { exact: true });
  const saveKey = page.getByRole("button", {
    name: "Save API key",
    exact: true,
  });
  await key.fill("owned-first-test-key");
  await expect(saveKey).toBeEnabled();
  await saveKey.click();
  await expect(key).toHaveValue("");
  await key.fill("draft-for-old-endpoint");
  await endpoint.fill("https://other.example.test/");
  await expect(saveKey).toBeDisabled();
  await saveKey.evaluate((button) =>
    button
      .closest("form")!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  await saveEndpoint.click();
  await expect(endpoint).toHaveValue("https://other.example.test");
  await expect(key).toHaveValue("");
  await expect(saveKey).toBeDisabled();
  await key.fill("owned-second-test-key");
  await expect(saveKey).toBeEnabled();
  await saveKey.click();
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
  await dialog
    .getByRole("combobox", { name: "Router", exact: true })
    .selectOption("claude-native");
  const mode = dialog.getByRole("combobox", {
    name: "Execution mode",
    exact: true,
  });
  await expect(mode.locator('option[value="native"]')).toHaveCount(1);
  await expect(mode).toHaveValue("");
  await mode.selectOption("native");
  await expect(dialog).toContainText("Permission requests appear in Lomi");
  const model = runField(dialog, "Model");
  expect(await model.evaluate((element) => element.tagName)).toBe("INPUT");
  await model.fill("claude-exact-native-model");
  await expect(
    dialog.getByRole("button", {
      name: "Open native account terminal",
      exact: true,
    }),
  ).toBeVisible();
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
  const outer = page.getByRole("dialog", {
    name: "Routed CLI runs",
    exact: true,
  });
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
