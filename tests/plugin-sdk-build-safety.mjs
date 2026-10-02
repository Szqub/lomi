import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import {
  mkdtemp,
  mkdir,
  writeFile,
  readFile,
  readdir,
  rm,
  symlink,
  rename,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildPlugin } from "@lomi-dev/plugin-sdk/build";
import { packageFiles, validatePackage } from "@lomi-dev/plugin-sdk/package";

async function project(t) {
  const root = await mkdtemp(join(tmpdir(), "lomi-build-safety-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const cwd = join(root, "author");
  await mkdir(join(cwd, "src"), { recursive: true });
  await writeFile(
    join(cwd, "src/index.tsx"),
    "export function activate() {}\n",
  );
  await writeFile(
    join(cwd, "plugin.json"),
    JSON.stringify({
      schemaVersion: 1,
      hostApi: 1,
      id: "example.test",
      name: "Test",
      description: "A plugin",
      version: "1.0.0",
      entry: "dist/index.js",
    }),
  );
  return { root, cwd };
}

test("external outputs are rejected before creating or replacing anything", async (t) => {
  const { root, cwd } = await project(t);
  const sibling = join(root, "sibling");
  await mkdir(sibling);
  await writeFile(join(sibling, "keep.txt"), "keep");
  for (const outDir of ["../sibling", sibling, "../missing/nested"]) {
    await assert.rejects(buildPlugin({ cwd, outDir }), /inside the project/);
    assert.equal(await readFile(join(sibling, "keep.txt"), "utf8"), "keep");
  }
  assert.deepEqual((await readdir(root)).sort(), ["author", "sibling"]);
  assert.deepEqual((await readdir(cwd)).sort(), ["plugin.json", "src"]);
});

test("symbolic link ancestors and output directories are rejected", async (t) => {
  const { root, cwd } = await project(t);
  const external = join(root, "external");
  await mkdir(external);
  await writeFile(join(external, "keep.txt"), "keep");
  await symlink(external, join(cwd, "linked"), "dir");
  for (const outDir of ["linked", "linked/nested/package"]) {
    await assert.rejects(buildPlugin({ cwd, outDir }), /real directories/);
  }
  assert.deepEqual(await readdir(external), ["keep.txt"]);
});

test("existing unrelated directories and valid but unowned packages survive", async (t) => {
  const { cwd } = await project(t);
  await mkdir(join(cwd, "tests"));
  await writeFile(join(cwd, "tests/keep.txt"), "keep");
  await assert.rejects(buildPlugin({ cwd, outDir: "tests" }), /not owned/);
  assert.equal(await readFile(join(cwd, "tests/keep.txt"), "utf8"), "keep");
  await buildPlugin({ cwd });
  await rm(join(cwd, "package.lomi-owned.json"));
  const before = await readFile(join(cwd, "package/dist/index.js"), "utf8");
  await assert.rejects(buildPlugin({ cwd }), /not owned/);
  assert.equal(
    await readFile(join(cwd, "package/dist/index.js"), "utf8"),
    before,
  );
});

test("the default package directory preserves unrelated existing files", async (t) => {
  const { cwd } = await project(t);
  await mkdir(join(cwd, "package"));
  await writeFile(join(cwd, "package/valuable.txt"), "preserve");
  await assert.rejects(buildPlugin({ cwd }), /not owned/);
  assert.deepEqual(await readdir(join(cwd, "package")), ["valuable.txt"]);
  assert.equal(
    await readFile(join(cwd, "package/valuable.txt"), "utf8"),
    "preserve",
  );
});

test("first and repeat builds keep ownership outside package files", async (t) => {
  const { cwd } = await project(t);
  const output = await buildPlugin({ cwd, outDir: "build/package" });
  const marker = `${output}.lomi-owned.json`;
  const firstOwner = await readFile(marker, "utf8");
  await writeFile(
    join(cwd, "src/index.tsx"),
    "export function activate() { return 42; }\n",
  );
  assert.equal(await buildPlugin({ cwd, outDir: "build/package" }), output);
  assert.notEqual(await readFile(marker, "utf8"), firstOwner);
  assert.match(await readFile(join(output, "dist/index.js"), "utf8"), /42/);
  await validatePackage(output);
  assert.deepEqual(
    [...(await packageFiles(output))].map(([path]) => path).sort(),
    ["dist/index.js", "dist/index.js.map", "plugin.json"],
  );
  assert.deepEqual((await readdir(join(cwd, "build"))).sort(), [
    "package",
    "package.lomi-owned.json",
  ]);
});

test("failed and cancelled rebuilds retain the last good output and marker", async (t) => {
  const { cwd } = await project(t);
  const output = await buildPlugin({ cwd });
  const marker = `${output}.lomi-owned.json`;
  const before = await readFile(join(output, "dist/index.js"), "utf8");
  const owner = await readFile(marker, "utf8");
  await writeFile(join(cwd, "src/index.tsx"), "invalid source !!!");
  await assert.rejects(buildPlugin({ cwd }));
  await writeFile(
    join(cwd, "src/index.tsx"),
    "export function activate() { return 42; }",
  );
  let checks = 0;
  await assert.rejects(
    buildPlugin({
      cwd,
      signal: {
        throwIfAborted() {
          if (++checks === 2) throw new Error("cancelled");
        },
      },
    }),
    /cancelled/,
  );
  assert.equal(await readFile(join(output, "dist/index.js"), "utf8"), before);
  assert.equal(await readFile(marker, "utf8"), owner);
  assert.deepEqual((await readdir(cwd)).sort(), [
    "package",
    "package.lomi-owned.json",
    "plugin.json",
    "src",
  ]);
  await buildPlugin({ cwd });
});

test("ownership cannot be reused for a different directory", async (t) => {
  const { cwd } = await project(t);
  const output = await buildPlugin({ cwd });
  await rename(output, join(cwd, "saved"));
  await mkdir(output);
  await writeFile(join(output, "keep.txt"), "keep");
  await assert.rejects(buildPlugin({ cwd }), /not owned/);
  assert.equal(await readFile(join(output, "keep.txt"), "utf8"), "keep");
});

test("a failed ownership commit rolls back output and retains the old marker", async (t) => {
  const { cwd } = await project(t);
  const output = await buildPlugin({ cwd });
  const marker = `${output}.lomi-owned.json`;
  const before = await readFile(join(output, "dist/index.js"), "utf8");
  const owner = await readFile(marker, "utf8");
  const originalRename = fs.renameSync;
  t.mock.method(fs, "renameSync", (from, to) => {
    if (to === marker) throw new Error("ownership commit failed");
    return originalRename(from, to);
  });
  syncBuiltinESMExports();
  try {
    await assert.rejects(buildPlugin({ cwd }), /ownership commit failed/);
  } finally {
    t.mock.restoreAll();
    syncBuiltinESMExports();
  }
  assert.equal(await readFile(join(output, "dist/index.js"), "utf8"), before);
  assert.equal(await readFile(marker, "utf8"), owner);
  await buildPlugin({ cwd });
});

test("sources imported from owned output are retained when the build is rejected", async (t) => {
  const { cwd } = await project(t);
  const output = await buildPlugin({ cwd });
  await writeFile(join(output, "source.js"), "export const value = 42;");
  await writeFile(
    join(cwd, "src/index.tsx"),
    "export { value } from '../package/source.js';",
  );
  await assert.rejects(buildPlugin({ cwd }), /contains source input/);
  assert.equal(
    await readFile(join(output, "source.js"), "utf8"),
    "export const value = 42;",
  );
});

test("outputs cannot contain manifest, source or asset inputs", async (t) => {
  const { cwd } = await project(t);
  await mkdir(join(cwd, "assets"));
  await writeFile(join(cwd, "assets/icon.svg"), "<svg/>");
  for (const outDir of [".", "src", "assets", "assets/generated"]) {
    await assert.rejects(
      buildPlugin({ cwd, outDir, assets: ["assets"] }),
      /separate/,
    );
  }
  assert.equal(await readFile(join(cwd, "assets/icon.svg"), "utf8"), "<svg/>");
});

test("the patched bundler preserves mjs entries, relative chunks and source-free maps", async (t) => {
  const { cwd } = await project(t);
  const manifestPath = join(cwd, "plugin.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  manifest.entry = "dist/activate.mjs";
  await writeFile(manifestPath, JSON.stringify(manifest));
  await writeFile(
    join(cwd, "src/index.tsx"),
    "export async function activate() { return import('./lazy.ts'); }",
  );
  await writeFile(join(cwd, "src/lazy.ts"), "export const value: number = 42;");
  const output = await buildPlugin({ cwd });
  const { files } = await validatePackage(output);
  assert.ok(files.has("dist/activate.mjs"));
  assert.ok(
    [...files.keys()].some(
      (path) => path !== "dist/activate.mjs" && path.endsWith(".mjs"),
    ),
  );
  for (const [path, bytes] of files) {
    if (path.endsWith(".map"))
      assert.equal(JSON.parse(bytes).sourcesContent, undefined);
  }
});
