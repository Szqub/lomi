import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { classifyTerminalInput } from "../src/terminal-input-source.ts";
const require = createRequire(
  new URL("../packages/remote-terminal-runtime/package.json", import.meta.url),
);
const { Terminal } = require("@xterm/headless");
test("actual xterm parser replies preserve source while typed input preempts", async () => {
  const terminal = new Terminal({ cols: 20, rows: 5, allowProposedApi: true }),
    received: Array<{ data: string; human: boolean }> = [];
  const classified = classifyTerminalInput(terminal, (data, human) =>
    received.push({ data, human }),
  );
  try {
    await new Promise<void>((resolve) =>
      terminal.write(new TextEncoder().encode("\x1b[6n\x1b[c"), resolve),
    );
    assert.equal(received.length, 2);
    assert.equal(
      received.every((v) => !v.human),
      true,
    );
    terminal._core.coreService.triggerDataEvent("local key", true);
    assert.deepEqual(received.at(-1), { data: "local key", human: true });
    terminal._core.coreService.triggerDataEvent("reply");
    assert.equal(received.at(-1)?.human, false);
    classified.dispose();
    terminal._core.coreService.triggerDataEvent("after disposal", true);
    assert.equal(received.length, 4);
  } finally {
    classified.dispose();
    terminal.dispose();
  }
});
