import assert from "node:assert/strict";
import { test } from "node:test";
import { newBrowserTab, newTab, newWorkspace, panes } from "../src/model.ts";
import type { Split } from "../src/model.ts";
import { summarizeWorkspaceAgents } from "../src/workspace-summary.ts";

test("summaries count detected agents across tabs, mixed splits, and separate workspaces", () => {
  const workspace = newWorkspace("/project", "bash");
  const first = newTab("/project", "bash", "Agents", 3);
  const second = newTab("/project", "bash", "Other");
  const browser = newBrowserTab();
  const leaves = panes(first.layout);
  first.layout = {
    type: "split",
    id: "mixed",
    axis: "horizontal",
    ratio: 0.5,
    first: first.layout,
    second: browser,
  };
  workspace.tabs = [first, second, newBrowserTab()];
  const other = newWorkspace("/other", "bash");
  const otherPane = panes((other.tabs[0] as typeof first).layout)[0];
  const secondPane = panes(second.layout)[0];
  const agents = {
    [leaves[0].id]: "codex",
    [leaves[1].id]: "claude",
    [secondPane.id]: "codex",
    [browser.id]: "gemini",
    [otherPane.id]: "gemini",
    stale: "claude",
  } as const;
  assert.deepEqual(summarizeWorkspaceAgents(workspace, agents), {
    total: 3,
    groups: [
      { cli: "codex", count: 2 },
      { cli: "claude", count: 1 },
    ],
    byTab: {
      [first.id]: [
        { cli: "codex", count: 1 },
        { cli: "claude", count: 1 },
      ],
      [second.id]: [{ cli: "codex", count: 1 }],
      [workspace.tabs[2].id]: [],
    },
  });
  assert.equal(summarizeWorkspaceAgents(other, agents).total, 1);
  assert.equal(summarizeWorkspaceAgents(workspace, {}).total, 0);
});

test("repeated pane IDs are counted once across split layouts and tabs", () => {
  const workspace = newWorkspace("/project", "bash");
  const tab = newTab("/project", "bash");
  const pane = panes(tab.layout)[0];
  tab.layout = {
    type: "split",
    id: "duplicate",
    axis: "vertical",
    ratio: 0.5,
    first: pane,
    second: pane,
  } satisfies Split;
  workspace.tabs = [tab, { ...tab, id: "duplicate-tab" }];
  const summary = summarizeWorkspaceAgents(workspace, { [pane.id]: "codex" });
  assert.equal(summary.total, 1);
  assert.deepEqual(summary.byTab[tab.id], [{ cli: "codex", count: 1 }]);
  assert.deepEqual(summary.byTab["duplicate-tab"], []);
});
