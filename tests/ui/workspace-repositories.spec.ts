import { expect, test } from "@playwright/test";

test("workspace repository scans share roots, retain incomplete counts and bound refreshes", async ({
  page,
}) => {
  await page.clock.install();
  await page.route("**/workspace-repository-harness", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: '<div id="root"></div>',
    }),
  );
  await page.addInitScript(() => {
    const state = window as any;
    state.isTauri = true;
    state.requests = [];
    state.__TAURI_INTERNALS__ = {
      invoke(command: string, args: { root: string }) {
        if (command !== "git_repositories") throw new Error(command);
        return new Promise((resolve, reject) => {
          state.requests.push({ root: args.root, resolve, reject });
        });
      },
    };
  });
  await page.goto("/workspace-repository-harness");
  await page.evaluate(async () => {
    const state = window as any;
    const reactPath = "/node_modules/.vite/deps/react.js";
    const domPath = "/node_modules/.vite/deps/react-dom_client.js";
    const hookPath = "/src/useWorkspaceRepositories.ts";
    const React = (await import(reactPath)).default;
    const { createRoot } = (await import(domPath)).default;
    const { default: useWorkspaceRepositories } = await import(hookPath);
    const root = createRoot(document.getElementById("root")!);
    state.projects = ["/active", "/a", "/a", "/b", "/c"].map((path, index) => ({
      id: String(index),
      path,
      workspaces: [],
    }));
    state.activeRoot = "/active";
    state.activeScan = {
      repositories: [{ root: "/active/repo", branch: "main", changes: [] }],
      errors: [],
      limited: false,
      loading: false,
    };
    function Harness() {
      state.summaries = useWorkspaceRepositories(
        state.projects,
        state.activeRoot,
        state.activeScan,
      );
      return null;
    }
    state.rerender = () => root.render(React.createElement(Harness));
    state.rerender();
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).requests.map((r: any) => r.root)),
    )
    .toEqual(["/a", "/b"]);
  await expect
    .poll(() => page.evaluate(() => (window as any).summaries["/active"].count))
    .toBe(1);
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).summaries["/active"].branch),
    )
    .toBe("main");
  await page.evaluate(() => {
    const state = window as any;
    state.activeScan = {
      ...state.activeScan,
      repositories: [
        { root: "/active/nested", branch: "nested", changes: [] },
        { root: "/active", branch: "root-branch", changes: [] },
      ],
    };
    state.rerender();
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).summaries["/active"].branch),
    )
    .toBe("root-branch");
  await page.evaluate(() => {
    const state = window as any;
    state.activeScan = {
      ...state.activeScan,
      repositories: [
        { root: "/active/first", branch: "first", changes: [] },
        { root: "/active/second", branch: "second", changes: [] },
      ],
    };
    state.rerender();
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).summaries["/active"].branch),
    )
    .toBeUndefined();
  await page.evaluate(() => {
    const state = window as any;
    state.requests[0].resolve({
      repositories: ["/a/first", "/a/second"].map((root) => ({
        root,
        branch: "main",
        changes: [],
      })),
      errors: [],
      limited: false,
    });
    state.requests[1].reject(new Error("Unavailable folder"));
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).requests.length))
    .toBe(3);
  await page.evaluate(() =>
    (window as any).requests[2].resolve({
      repositories: [],
      errors: [],
      limited: false,
    }),
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).summaries["/b"]))
    .toEqual({
      count: 0,
      branch: undefined,
      loading: false,
      limited: false,
      error: "Unavailable folder",
    });
  await expect
    .poll(() => page.evaluate(() => (window as any).summaries["/c"].loading))
    .toBe(false);

  await page.evaluate(() => {
    const state = window as any;
    state.projects = state.projects.map((project: any) => ({
      ...project,
      name: "Renamed",
      workspaces: [{ tabs: [] }],
    }));
    state.rerender();
  });
  await page.clock.runFor(100);
  expect(await page.evaluate(() => (window as any).requests.length)).toBe(3);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect
    .poll(() => page.evaluate(() => (window as any).requests.length))
    .toBe(5);
  await page.evaluate(() => {
    const state = window as any;
    state.requests[3].resolve({ repositories: [], errors: [], limited: true });
    state.requests[4].resolve({ repositories: [], errors: [], limited: false });
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).requests.length))
    .toBe(6);
  await page.evaluate(() =>
    (window as any).requests[5].resolve({
      repositories: [],
      errors: [],
      limited: false,
    }),
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).summaries["/a"]))
    .toEqual({
      count: 2,
      branch: undefined,
      loading: false,
      limited: true,
      error: undefined,
    });

  await page.evaluate(() =>
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    }),
  );
  await page.clock.fastForward(60_000);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  expect(await page.evaluate(() => (window as any).requests.length)).toBe(6);
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).requests.length))
    .toBe(8);
  await page.evaluate(() => {
    const state = window as any;
    state.projects = state.projects.filter(
      (project: any) => project.path !== "/a",
    );
    state.rerender();
  });
  await page.clock.runFor(100);
  await page.evaluate(() => {
    const state = window as any;
    state.requests[6].resolve({
      repositories: [{ root: "/a/stale", branch: "main", changes: [] }],
      errors: [],
      limited: false,
    });
    state.requests[7].resolve({ repositories: [], errors: [], limited: false });
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).requests.length))
    .toBe(10);
  expect(
    await page.evaluate(() => (window as any).summaries["/a"]),
  ).toBeUndefined();
  expect(
    await page.evaluate(() =>
      (window as any).requests.slice(8).map((request: any) => request.root),
    ),
  ).toEqual(["/b", "/c"]);
  await page.evaluate(() => {
    const state = window as any;
    for (const request of state.requests.slice(8)) {
      request.resolve({ repositories: [], errors: [], limited: false });
    }
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).summaries["/c"].loading))
    .toBe(false);
  await page.clock.fastForward(60_000);
  await expect
    .poll(() => page.evaluate(() => (window as any).requests.length))
    .toBe(12);
  await page.evaluate(() => {
    const state = window as any;
    for (const request of state.requests.slice(10)) {
      request.resolve({ repositories: [], errors: [], limited: false });
    }
    state.activeRoot = "/b";
    state.activeScan = {
      repositories: [{ root: "/b/repo", branch: "main", changes: [] }],
      errors: [],
      limited: false,
      loading: false,
    };
    state.rerender();
  });
  await expect
    .poll(() => page.evaluate(() => (window as any).requests.length))
    .toBe(14);
  expect(
    await page.evaluate(() =>
      (window as any).requests.slice(12).map((request: any) => request.root),
    ),
  ).toEqual(["/active", "/c"]);
  expect(await page.evaluate(() => (window as any).summaries["/b"].count)).toBe(
    1,
  );
});
