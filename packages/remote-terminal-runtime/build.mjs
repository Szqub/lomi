import { build } from "esbuild";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
const dir = fileURLToPath(new URL(".", import.meta.url)),
  out = new URL("../../src-tauri/resources/remote-terminal/", import.meta.url);
await mkdir(out, { recursive: true });
const outfile = fileURLToPath(new URL("index.cjs", out));
await build({
  absWorkingDir: dir,
  entryPoints: ["src/index.mjs"],
  outfile,
  bundle: true,
  platform: "node",
  target: "node24",
  format: "cjs",
  legalComments: "external",
  sourcemap: false,
});
await writeFile(
  outfile + ".sha256",
  createHash("sha256")
    .update(await readFile(outfile))
    .digest("hex") + "\n",
);
const license = await readFile(
  new URL("../../node_modules/@xterm/xterm/LICENSE", import.meta.url),
  "utf8",
);
await writeFile(
  new URL("THIRD-PARTY-NOTICES", out),
  "@xterm/headless 6.0.0 (MIT)\n" + license + "\n",
);
