import assert from "node:assert/strict";
import { test } from "node:test";
import { newProject, newSession, restoreSession } from "../src/model.ts";
import type { AppInfo } from "../src/model.ts";
import {
  MAX_WORKSPACE_IMAGE_BYTES,
  sanitizeWorkspaceAppearance,
  WORKSPACE_ICONS,
} from "../src/workspace-appearance.ts";

const info: AppInfo = {
  directory: "/project",
  home: "/home/test",
  platform: "linux",
  profiles: [],
};
const image = "data:image/png;base64,iVBORw0KGgo=";

test("appearance survives serialized restoration without changing legacy workspaces", () => {
  const project = newProject("/project", "bash");
  const session = {
    ...newSession(),
    projects: [project],
    activeProjectId: project.id,
  };
  const legacy = restoreSession(JSON.parse(JSON.stringify(session)), info);
  assert.equal(
    Object.hasOwn(legacy.projects[0].workspaces[0], "appearance"),
    false,
  );
  project.workspaces[0].appearance = { icon: "🚀", color: "#aAbB00", image };
  assert.deepEqual(
    restoreSession(JSON.parse(JSON.stringify(session)), info).projects[0]
      .workspaces[0].appearance,
    project.workspaces[0].appearance,
  );
});

test("restoration drops invalid appearance fields while retaining valid independent fields", () => {
  const project = newProject("/project", "bash");
  const serialized = JSON.parse(
    JSON.stringify({ ...newSession(), projects: [project] }),
  );
  serialized.projects[0].workspaces[0].appearance = {
    icon: "x".repeat(17),
    color: "#123456",
    image: "data:image/svg+xml;base64,PHN2Zz4=",
    extra: true,
  };
  assert.deepEqual(
    restoreSession(serialized, info).projects[0].workspaces[0].appearance,
    { color: "#123456" },
  );
  for (const invalid of [
    null,
    [],
    12,
    "folder",
    {},
    { icon: "\n", color: "red", image: "https://example.com/a.png" },
  ])
    assert.equal(sanitizeWorkspaceAppearance(invalid), undefined);
});

test("icons allow catalog values and bounded custom Unicode text", () => {
  for (const icon of [...WORKSPACE_ICONS, "Team", "🚀".repeat(16)])
    assert.deepEqual(sanitizeWorkspaceAppearance({ icon }), { icon });
  assert.equal(
    sanitizeWorkspaceAppearance({ icon: "🚀".repeat(17) }),
    undefined,
  );
  assert.equal(sanitizeWorkspaceAppearance({ icon: "a\u0000b" }), undefined);
});

test("images require bounded base64 raster data URLs", () => {
  const bytes = Buffer.alloc(MAX_WORKSPACE_IMAGE_BYTES);
  for (const mime of ["png", "jpeg", "webp"]) {
    const value = `data:image/${mime};base64,${bytes.toString("base64")}`;
    assert.deepEqual(sanitizeWorkspaceAppearance({ image: value }), {
      image: value,
    });
  }
  for (const value of [
    `data:image/png;base64,${Buffer.alloc(MAX_WORKSPACE_IMAGE_BYTES + 1).toString("base64")}`,
    "data:image/gif;base64,AAAA",
    "data:image/png;base64,A===",
    "data:image/png;base64,AAA",
    "data:image/png,abc",
  ])
    assert.equal(sanitizeWorkspaceAppearance({ image: value }), undefined);
});
