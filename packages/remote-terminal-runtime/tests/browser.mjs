import assert from "node:assert/strict";
import { build } from "esbuild";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
const directory = fileURLToPath(new URL("..", import.meta.url));
const result = await build({
  absWorkingDir: directory,
  stdin: {
    contents: `import {Terminal} from '@xterm/xterm';import{captureSnapshot,restoreSnapshot,installTerminalProfile,assertModelBounds}from'./src/snapshot.mjs';window.fixture={Terminal,captureSnapshot,restoreSnapshot,installTerminalProfile,assertModelBounds};`,
    resolveDir: directory,
  },
  bundle: true,
  platform: "browser",
  format: "iife",
  write: false,
});
const server = createServer((req, res) => {
  if (req.url === "/bundle.js") {
    res.setHeader("content-type", "text/javascript");
    res.end(result.outputFiles[0].text);
  } else {
    res.setHeader("content-type", "text/html");
    res.end(
      '<!doctype html><div id="a"></div><div id="b"></div><script src="/bundle.js"></script>',
    );
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  const checks = await page.evaluate(async () => {
    const {
      Terminal,
      captureSnapshot,
      restoreSnapshot,
      installTerminalProfile,
    } = window.fixture;
    let checks = 0;
    const write = (t, bytes) =>
      new Promise((r) => t.write(new TextEncoder().encode(bytes), r));
    for (const [prefix, suffix] of [
      [
        "\x1b]4;1;rgb:00/ff/00\x07\x1b]10;rgb:00/00/ff\x07\x1b[31mGREEN",
        "\x1b[31mFUTURE\x1b[6n",
      ],
      [
        "你好e\u0301\x1b[4:3m\x1b[58:2::1:2:3mUNDERLINE\x1b[?1049hALT\x1b[2;4r\x1b[?6h\x1b7",
        "\x1b8future\x1b[?1049lEND",
      ],
      ["\x1b(0\x1b7\x1b(B\x1b8", "q"],
      [
        Array.from(
          { length: 300 },
          (_, i) => `line${i}abcdefghijklmnopqrst\r\n`,
        ).join(""),
        "future\r\n",
      ],
    ]) {
      const a = new Terminal({
          cols: 20,
          rows: 5,
          scrollback: 200,
          allowProposedApi: true,
          logLevel: "off",
        }),
        b = new Terminal({
          cols: 20,
          rows: 5,
          scrollback: 200,
          allowProposedApi: true,
          logLevel: "off",
        });
      installTerminalProfile(a);
      installTerminalProfile(b);
      a.open(document.querySelector("#a"));
      b.open(document.querySelector("#b"));
      let ar = [],
        br = [];
      a.onData((v) => ar.push(v));
      b.onData((v) => br.push(v));
      await write(a, prefix);
      ar = [];
      restoreSnapshot(b, JSON.parse(JSON.stringify(captureSnapshot(a))));
      if (
        JSON.stringify(captureSnapshot(a)) !==
        JSON.stringify(captureSnapshot(b))
      )
        throw Error("browser snapshot parity");
      checks++;
      const color = (t) => {
        const c = t._core._themeService.colors;
        return JSON.stringify({
          ansi: c.ansi.map((v) => v.css),
          fg: c.foreground.css,
          bg: c.background.css,
          cursor: c.cursor.css,
        });
      };
      if (color(a) !== color(b)) throw Error("fixed palette mismatch");
      checks++;
      await write(a, suffix);
      await write(b, suffix);
      if (
        JSON.stringify(captureSnapshot(a)) !==
        JSON.stringify(captureSnapshot(b))
      )
        throw Error("browser future output parity");
      if (JSON.stringify(ar) !== JSON.stringify(br))
        throw Error("terminal reply parity");
      checks++;
      await new Promise((r) => requestAnimationFrame(r));
      if (
        a.element.querySelector(".xterm-rows").textContent !==
        b.element.querySelector(".xterm-rows").textContent
      )
        throw Error("rendered rows mismatch");
      checks++;
      a.dispose();
      b.dispose();
    }
    return checks;
  });
  assert.equal(checks, 16);
  console.log(
    JSON.stringify({
      kind: "remote_snapshot_browser",
      engine: "Chromium",
      checks,
      status: "passed",
    }),
  );
} finally {
  await browser?.close();
  await new Promise((r) => server.close(r));
}
