import assert from "node:assert/strict";
import { test } from "node:test";
import {
  active,
  addWorkspace,
  newSession,
  newTab,
  newBrowserTab,
  newPane,
  panes,
} from "../src/model.ts";
import {
  WorkspacePublisher,
  workspaceTerminals,
} from "../src/remote-workspace-domain.ts";
import type { WorkspaceProjection } from "../src/remote-workspace-domain.ts";
import type { Session, ShellProfile } from "../src/model.ts";
const profiles: ShellProfile[] = [
  {
    id: "bash",
    name: "bash",
    kind: "bash",
    program: "/bin/bash",
    distro: null,
    home: "/home/test",
  },
  {
    id: "zsh",
    name: "zsh",
    kind: "zsh",
    program: "/bin/zsh",
    distro: null,
    home: "/home/test",
  },
];
const deferred = () => {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  return { resolve, promise };
};
function fixture(start?: (id: string) => Promise<void>) {
  let session: Session = addWorkspace(
    newSession(),
    "/project",
    "bash",
    "Hidden",
  );
  const workspace = active(session)!.workspace;
  workspace.tabs.push(
    newTab("/project", "bash", "Inactive", 2),
    newBrowserTab(),
  );
  const terminal = workspaceTerminals(session)[0].terminals[1].pane;
  terminal.profileId = "zsh";
  session = addWorkspace(session, "/other", "bash", "Unshared");
  const ids = new Map<string, string>();
  const started: [string, string][] = [];
  const publications: {
    revision: number;
    workspaces: WorkspaceProjection[];
  }[] = [];
  const engine = new WorkspacePublisher({
    session: () => session,
    profiles: () => profiles,
    sessionId: (id) => ids.get(id) ?? null,
    begin: async () => ({ epoch: "epoch" }),
    sync: async (_, revision, workspaces) => {
      publications.push({ revision, workspaces });
    },
    start: async (pane, profile) => {
      if (ids.has(pane.id)) return;
      started.push([pane.id, profile.id]);
      await start?.(pane.id);
      ids.set(pane.id, `session-${pane.id}`);
    },
  });
  engine.desired.add(workspace.id);
  return {
    engine,
    workspace,
    started,
    publications,
    session: () => session,
    replace: (next: Session) => (session = next),
  };
}
test("sharing starts every hidden terminal with its own profile and excludes nonterminal/unshared tabs", async () => {
  const f = fixture();
  await f.engine.notify();
  assert.equal(f.started.length, 3);
  assert.equal(f.started[1][1], "zsh");
  const latest = f.publications.at(-1)!;
  assert.equal(latest.workspaces[0].terminals.length, 3);
  assert.ok(latest.workspaces[0].terminals.every((t) => t.sessionId));
  assert.equal(latest.workspaces[1].terminals[0].sessionId, null);
  assert.equal(latest.workspaces[0].terminals[1].title, "Inactive");
  assert.deepEqual(
    f.publications.map((p) => p.revision),
    [1, 2],
  );
});
test("a deleted workspace during asynchronous startup cannot be resurrected or start remaining terminals", async () => {
  const gate = deferred();
  const f = fixture(() => gate.promise);
  const pending = f.engine.notify();
  await new Promise((r) => setImmediate(r));
  f.replace({ ...f.session(), projects: f.session().projects.slice(1) });
  void f.engine.notify();
  gate.resolve();
  await pending;
  assert.equal(f.started.length, 1);
  assert.deepEqual(
    f.publications.at(-1)!.workspaces.map((w) => w.id),
    [f.session().projects[0].workspaces[0].id],
  );
});
test("unsharing while starting prevents further background starts", async () => {
  const gate = deferred();
  const f = fixture(() => gate.promise);
  const pending = f.engine.notify();
  await new Promise((r) => setImmediate(r));
  f.engine.desired.clear();
  void f.engine.notify();
  gate.resolve();
  await pending;
  assert.equal(f.started.length, 1);
});
test("moves publish only the latest membership after startup", async () => {
  const gate = deferred();
  const f = fixture(() => gate.promise);
  const pending = f.engine.notify();
  await new Promise((r) => setImmediate(r));
  const session = f.session();
  const moved = f.workspace.tabs[0];
  f.replace({
    ...session,
    projects: session.projects.map((p, index) => ({
      ...p,
      workspaces: p.workspaces.map((w) => ({
        ...w,
        tabs: index === 0 ? w.tabs.slice(1) : [...w.tabs, moved],
      })),
    })),
  });
  void f.engine.notify();
  gate.resolve();
  await pending;
  const final = f.publications.at(-1)!.workspaces;
  const pane = panes(
    moved.type === "terminal" ? moved.layout : newTab("/", "bash").layout,
  )[0].id;
  assert.ok(!final[0].terminals.some((t) => t.paneId === pane));
  assert.ok(final[1].terminals.some((t) => t.paneId === pane));
});

test("new hidden tabs, splits and restarted panes join the current shared workspace", async () => {
  const f = fixture();
  await f.engine.notify();
  const session = f.session();
  const added = newTab("/project", "bash", "New hidden", 2);
  const restarted = newPane("/project");
  f.replace({
    ...session,
    projects: session.projects.map((project, index) =>
      index
        ? project
        : {
            ...project,
            workspaces: project.workspaces.map((workspace) => ({
              ...workspace,
              tabs: workspace.tabs
                .map((tab, tabIndex) =>
                  tabIndex === 0 && tab.type === "terminal"
                    ? { ...tab, layout: restarted, activePaneId: restarted.id }
                    : tab,
                )
                .concat(added),
            })),
          },
    ),
  });
  await f.engine.notify();
  const final = f.publications.at(-1)!.workspaces[0].terminals;
  assert.equal(final.length, 5);
  assert.ok(final.every((terminal) => terminal.sessionId));
  assert.ok(final.some((terminal) => terminal.paneId === restarted.id));
  assert.ok(!final.some((terminal) => terminal.paneId === f.started[0][0]));
  assert.equal(f.started.length, 6);
});

test("projection preserves native tab identities and prunes mixed layouts", () => {
  const session = addWorkspace(newSession(), "/project", "bash", "Shared");
  const workspace = active(session)!.workspace;
  const first = newTab("/project", "bash", "Same title", 2);
  const second = newTab("/project", "bash", "Same title");
  const empty = newTab("/project", "bash", "Browser only");
  const original = first.layout;
  first.layout = {
    type: "split",
    id: "mixed",
    axis: "vertical",
    ratio: 0.2,
    first: newBrowserTab("https://private.example"),
    second: original,
  };
  empty.layout = newBrowserTab();
  workspace.tabs = [first, empty, newBrowserTab(), second];
  const projection = workspaceTerminals(session)[0];
  assert.deepEqual(
    projection.tabs.map((tab) => [tab.id, tab.title]),
    [
      [first.id, "Same title"],
      [second.id, "Same title"],
    ],
  );
  assert.equal(projection.tabs[0].layout.type, "split");
  if (original.type !== "split" || projection.tabs[0].layout.type !== "split")
    throw new Error("Expected split fixture");
  assert.equal(projection.tabs[0].layout.ratio, original.ratio);
  assert.equal(projection.terminals.length, 3);
  assert.ok(!JSON.stringify(projection.tabs).includes("private.example"));
});

test("layout-only changes publish metadata without restarting terminals", async () => {
  const f = fixture();
  await f.engine.notify();
  const tab = f.workspace.tabs[1];
  if (tab.type !== "terminal" || tab.layout.type !== "split")
    throw new Error("Expected split fixture");
  const previous = f.publications.length;
  const started = f.started.length;
  tab.layout = { ...tab.layout, axis: "vertical", ratio: 0.3 };
  await f.engine.notify();
  assert.equal(f.publications.length, previous + 1);
  assert.equal(f.started.length, started);
  const layout = f.publications.at(-1)!.workspaces[0].tabs![1].layout;
  assert.equal(layout.type, "split");
  if (layout.type === "split") {
    assert.equal(layout.axis, "vertical");
    assert.equal(layout.ratio, 0.3);
  }
});
