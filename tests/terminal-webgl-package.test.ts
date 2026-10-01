import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";

test("patched WebGL bundles load and dispose before activation", async () => {
  const require = createRequire(import.meta.url);
  const commonjs = require("@xterm/addon-webgl");
  const modulePath = join(
    dirname(require.resolve("@xterm/addon-webgl")),
    "addon-webgl.mjs",
  );
  const esm = await import(pathToFileURL(modulePath).href);
  for (const bundle of [commonjs, esm]) {
    assert.equal(typeof bundle.WebglAddon, "function");
    new bundle.WebglAddon().dispose();
  }
});
