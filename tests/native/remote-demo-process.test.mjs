import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { test } from "node:test";
import { terminateOwnedProcessGroup } from "./remote-demo-process.mjs";

function start(source) {
  const child = spawn(process.execPath, ["-e", source], {
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.detached = true;
  return child;
}
async function ready(child) {
  const [bytes] = await once(child.stdout, "data");
  assert.match(bytes.toString(), /READY/);
}
function absent(child) {
  assert.throws(() => process.kill(-child.pid, 0), { code: "ESRCH" });
}
async function cleanup(child) {
  await terminateOwnedProcessGroup(child, { graceMs: 100, forceMs: 5000 });
}

test("terminates a normal owned detached process group", async (t) => {
  const child = start("console.log('READY'); setInterval(() => {}, 1000)");
  t.after(() => cleanup(child));
  await ready(child);
  await terminateOwnedProcessGroup(child, { graceMs: 1000 });
  absent(child);
});

test("forces cleanup when the child ignores SIGTERM", async (t) => {
  const child = start(
    "process.on('SIGTERM', () => {}); console.log('READY'); setInterval(() => {}, 1000)",
  );
  t.after(() => cleanup(child));
  await ready(child);
  const exited = once(child, "exit");
  await terminateOwnedProcessGroup(child, { graceMs: 100 });
  assert.deepEqual(await exited, [null, "SIGKILL"]);
  absent(child);
});

test("cleans descendants after their wrapper has already exited", async (t) => {
  const child = start(`
    const { spawn } = require('node:child_process');
    const descendant = spawn(process.execPath, ['-e',
      "process.on('SIGTERM', () => {}); console.log('READY'); setInterval(() => {}, 1000)"],
      { stdio: ['ignore', 'pipe', 'ignore'] });
    descendant.stdout.once('data', () => { console.log('READY'); process.exit(0); });
  `);
  t.after(() => cleanup(child));
  const exited = once(child, "exit");
  await ready(child);
  assert.deepEqual(await exited, [0, null]);
  assert.doesNotThrow(() => process.kill(-child.pid, 0));
  await terminateOwnedProcessGroup(child, { graceMs: 100 });
  absent(child);
});

test("rejects unowned objects and unmarked children without signaling", async (t) => {
  await assert.rejects(
    terminateOwnedProcessGroup({ pid: process.pid, detached: true }),
  );
  const child = start("console.log('READY'); setInterval(() => {}, 1000)");
  t.after(() => cleanup(child));
  await ready(child);
  delete child.detached;
  await assert.rejects(
    terminateOwnedProcessGroup(child),
    /known detached child/,
  );
  child.detached = true;
  assert.doesNotThrow(() => process.kill(-child.pid, 0));
});
