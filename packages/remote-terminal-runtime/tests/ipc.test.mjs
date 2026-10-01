import test from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { validateSnapshot } from "../src/snapshot.mjs";
const sessionId = "11111111-1111-4111-8111-111111111111",
  epoch = "22222222-2222-4222-8222-222222222222";
function helper() {
  const child = spawn(process.execPath, ["src/index.mjs"], {
    cwd: new URL("..", import.meta.url),
    stdio: ["pipe", "pipe", "pipe"],
    env: { PATH: process.env.PATH },
  });
  const reader = createInterface({ input: child.stdout });
  const replies = [];
  reader.on("line", (v) => {
    const resolve = replies.shift();
    if (resolve) resolve(JSON.parse(v));
  });
  return {
    child,
    request: (v) =>
      new Promise((resolve) => {
        replies.push(resolve);
        child.stdin.write(JSON.stringify(v) + "\n");
      }),
  };
}
test("private subprocess processes ordered output and snapshot barriers", async () => {
  const h = helper();
  try {
    let r = await h.request({
      id: 1,
      op: "create",
      sessionId,
      epoch,
      seq: 0,
      cols: 20,
      rows: 5,
    });
    assert.equal(r.ok, true);
    r = await h.request({
      id: 2,
      op: "write",
      sessionId,
      epoch,
      seq: 1,
      data: Buffer.from("PTY zażółć界\r\n\x1b[31mRED").toString("base64"),
    });
    assert.equal(r.ok, true);
    r = await h.request({ id: 3, op: "snapshot", sessionId, epoch, seq: 1 });
    assert.equal(r.ok, true);
    validateSnapshot(r.result.state);
    assert.equal(r.result.throughSeq, 1);
    r = await h.request({
      id: 4,
      op: "write",
      sessionId,
      epoch: "33333333-3333-4333-8333-333333333333",
      seq: 2,
      data: "",
    });
    assert.equal(r.ok, false);
    assert.equal(r.error, "STALE_SESSION");
    r = await h.request({
      id: 5,
      op: "write",
      sessionId,
      epoch,
      seq: 4,
      data: "",
    });
    assert.equal(r.error, "STALE_SEQUENCE");
    h.child.stdin.end();
    await once(h.child, "exit");
    assert.equal(h.child.exitCode, 0);
  } finally {
    h.child.kill();
  }
});
test("oversized private IPC fails closed without exposing request bytes", async () => {
  const h = helper();
  let output = "";
  h.child.stdout.on("data", (v) => (output += v));
  h.child.stdin.on("error", () => {});
  h.child.stdin.write("X".repeat(65537));
  await once(h.child, "exit");
  assert.equal(h.child.exitCode, 1);
  assert.equal(output.includes("XXXX"), false);
});
