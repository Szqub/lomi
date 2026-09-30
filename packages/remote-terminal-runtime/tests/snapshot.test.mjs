import test from "node:test";
import assert from "node:assert/strict";
import headless from "@xterm/headless";
import {
  captureSnapshot,
  restoreSnapshot,
  safeBoundary,
  validateSnapshot,
  installTerminalProfile,
} from "../src/snapshot.mjs";
import { writeTerminal, TerminalModels } from "../src/model.mjs";
const make = () => {
  const t = new headless.Terminal({
    cols: 20,
    rows: 5,
    scrollback: 200,
    allowProposedApi: true,
    logLevel: "off",
  });
  installTerminalProfile(t);
  return t;
};
const text = (s) => new TextEncoder().encode(s);
async function roundtrip(prefix, suffix, resize) {
  const a = make(),
    b = make();
  try {
    await writeTerminal(a, text(prefix));
    if (resize) a.resize(...resize);
    assert.equal(safeBoundary(a), true);
    restoreSnapshot(b, JSON.parse(JSON.stringify(captureSnapshot(a))));
    assert.deepEqual(captureSnapshot(b), captureSnapshot(a));
    await writeTerminal(a, text(suffix));
    await writeTerminal(b, text(suffix));
    assert.deepEqual(captureSnapshot(b), captureSnapshot(a));
  } finally {
    a.dispose();
    b.dispose();
  }
}
test("saved cursors, alternate buffers, margins and future output", async () => {
  await roundtrip(
    "normal\x1b[31m\x1b7\x1b[?1049hALT\x1b[2;4r\x1b[?6h\x1b[2;3H\x1b7",
    "future\x1b8!\x1b[?1049l\x1b8!\r\nnext",
  );
});
test("CJK combined cells, extended SGR, tabs, modes and hidden cursor", async () => {
  await roundtrip(
    "你好e\u0301🙂\x1b[4:3m\x1b[58:2::1:2:3munderline\x1b[?25l\x1b[?1006h\x1b[?1003h\x1b[3g\x1b[5G\x1bH\x1b[4h\x1b[?7l",
    "\r\tTAB\x1b[0m\x1b[?25h\x1b[6n",
  );
});
test("resize/reflow with both buffers", async () => {
  await roundtrip(
    "abcdefghijklmnopqrstuvwxyz\r\n0123456789\x1b[?1049h1234567890abcdef",
    "XYZ\x1b[?1049lMORE",
    [12, 7],
  );
});
test("finite scrollback survives eviction with wrapping", async () => {
  await roundtrip(
    Array.from(
      { length: 300 },
      (_, i) => `line${i}abcdefghijklmnopqrstuvwxyz\r\n`,
    ).join(""),
    "future\r\n",
  );
});
test("dec special charset and title save/restore", async () => {
  await roundtrip(
    "\x1b(0lqk\x1b7\x1b]0;Title\x07\x1b[22;0t\x1b]0;Changed\x07",
    "qqq\x1b8q\x1b[23;0t",
  );
});
test("model snapshot preserves incomplete UTF8 and escape parser by exact suffix", async () => {
  const id = "11111111-1111-4111-8111-111111111111",
    epoch = "22222222-2222-4222-8222-222222222222";
  for (const payload of [
    "你好",
    "\x1b[31mRED",
    "\x1b]0;title\x07",
    "\x1bP$qm\x1b\\",
  ]) {
    const raw = text(payload);
    for (let split = 1; split < raw.length; split++) {
      const m = new TerminalModels();
      let i = 0;
      const req = (op, fields = {}) =>
        m.request({
          id: ++i,
          op,
          sessionId: id,
          epoch,
          seq: op === "write" ? 1 : 0,
          ...fields,
        });
      await req("create", { cols: 20, rows: 5 });
      await req("write", {
        data: Buffer.from(raw.subarray(0, split)).toString("base64"),
      });
      const s = await req("snapshot", { seq: 1 });
      const b = make();
      restoreSnapshot(b, s.state);
      await writeTerminal(b, Buffer.from(s.suffix, "base64"));
      await writeTerminal(b, raw.subarray(split));
      await m.request({
        id: ++i,
        op: "write",
        sessionId: id,
        epoch,
        seq: 2,
        data: Buffer.from(raw.subarray(split)).toString("base64"),
      });
      assert.deepEqual(
        captureSnapshot(b),
        captureSnapshot(m.sessions.get(id).term),
      );
      b.dispose();
      m.dispose();
    }
  }
});
test("malformed snapshot and stale helper epochs fail closed", async () => {
  const t = make();
  const s = captureSnapshot(t);
  s.normal.lines[0].cells = "!";
  assert.throws(() => validateSnapshot(s));
  t.dispose();
  const m = new TerminalModels();
  await assert.rejects(
    m.request({
      id: 1,
      op: "snapshot",
      sessionId: "11111111-1111-4111-8111-111111111111",
      epoch: "22222222-2222-4222-8222-222222222222",
      seq: 0,
    }),
    /STALE_SESSION/,
  );
  m.dispose();
});

test("restored cursor carries active charset independent of designated bank", async () => {
  await roundtrip("\x1b(0\x1b7\x1b(B\x1b8", "q");
});

test("legal large titles and combined cells produce consumable snapshots", async () => {
  await roundtrip("\x1b]0;" + "x".repeat(5000) + "\x07", "future");
  await roundtrip("a" + "\u0301".repeat(4097), "future");
});
test("retained combining accumulation reaches a deliberate per-model limit", async () => {
  const m = new TerminalModels(),
    sessionId = "11111111-1111-4111-8111-111111111111",
    epoch = "22222222-2222-4222-8222-222222222222";
  await m.request({
    id: 1,
    op: "create",
    sessionId,
    epoch,
    seq: 0,
    cols: 20,
    rows: 5,
  });
  const data = Buffer.from("a" + "\u0301".repeat(8000)).toString("base64");
  await m.request({ id: 2, op: "write", sessionId, epoch, seq: 1, data });
  await m.request({
    id: 3,
    op: "write",
    sessionId,
    epoch,
    seq: 2,
    data: Buffer.from("\u0301".repeat(8000)).toString("base64"),
  });
  await assert.rejects(
    m.request({
      id: 4,
      op: "write",
      sessionId,
      epoch,
      seq: 3,
      data: Buffer.from("\u0301".repeat(1000)).toString("base64"),
    }),
    /MODEL_LIMIT/,
  );
  await assert.rejects(
    m.request({ id: 5, op: "snapshot", sessionId, epoch, seq: 3 }),
    /MODEL_LIMIT/,
  );
  m.dispose();
});

test("near-limit state plus partial suffix cannot declare an oversized wire snapshot", async () => {
  const m = new TerminalModels(),
    sessionId = "11111111-1111-4111-8111-111111111111",
    epoch = "22222222-2222-4222-8222-222222222222";
  let seq = 0,
    id = 0;
  const request = (op, extra = {}) =>
    m.request({ id: ++id, op, sessionId, epoch, seq, ...extra });
  try {
    await request("create", { cols: 400, rows: 500 });
    seq++;
    await request("write", {
      data: Buffer.from("\n".repeat(700) + "\x1b[?1049h").toString("base64"),
    });
    await request("snapshot");
    for (let i = 0; i < 32; i++) {
      seq++;
      await request("write", {
        data: Buffer.from(
          (i === 0 ? "\x1bP" : "") + "x".repeat(i === 0 ? 16382 : 16384),
        ).toString("base64"),
      });
    }
    await assert.rejects(request("snapshot"), /SNAPSHOT_LIMIT/);
  } finally {
    m.dispose();
  }
});

test("ANSI LNM and DEC cursor blinking modes survive future line feeds", async () => {
  await roundtrip("\x1b[20habc\x1b[?12h", "\nx\x1b[?12l\nend");
});
