import assert from "node:assert/strict";
import test from "node:test";
import { spawn } from "node:child_process";
import { readFile, access } from "node:fs/promises";
import { join, resolve } from "node:path";
import { homedir } from "node:os";

const root = resolve(import.meta.dirname, "../..");
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
function launch() {
  const child = spawn(
    process.execPath,
    [
      "--experimental-strip-types",
      "tests/native/run-remote-live.mjs",
      "--manual",
    ],
    {
      cwd: root,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  const events = [];
  let buffer = "";
  child.stdout.on("data", (bytes) => {
    buffer += bytes.toString();
    const lines = buffer.split("\n");
    buffer = lines.pop();
    for (const line of lines) {
      try {
        events.push(JSON.parse(line));
      } catch {}
    }
  });
  child.stderr.resume();
  return { child, events };
}
async function wait(fn, timeout = 90000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await fn();
    if (value) return value;
    await delay(50);
  }
  throw Error("Local demo startup verification timed out.");
}
async function stop(run) {
  if (run.child.exitCode === null) run.child.kill("SIGINT");
  await wait(() => run.child.exitCode !== null, 25000);
}
async function assertClean(event) {
  await assert.rejects(
    access(join(homedir(), "Library/Application Support", event.identifier)),
    { code: "ENOENT" },
  );
  await assert.rejects(access(join(event.artifactDirectory, "fixture.json")), {
    code: "ENOENT",
  });
}

test("Ctrl+C while building cleans the owned application and credential copy", async () => {
  const run = launch();
  try {
    const event = await wait(() =>
      run.events.find((event) => event.status === "building"),
    );
    await stop(run);
    assert.equal(run.child.exitCode, 0);
    assert.equal(
      run.events.some((event) => event.ready),
      false,
    );
    await assertClean(event);
  } finally {
    await stop(run);
  }
});

test("concurrent launch leaves the first demo intact; Ctrl+C after browser readiness cleans it", async () => {
  const first = launch();
  let second;
  try {
    const started = await wait(() =>
      first.events.find((event) => event.status === "building"),
    );
    await wait(() => first.events.find((event) => event.ready));
    const state = await readFile(
      join(started.artifactDirectory, "native-state.json"),
      "utf8",
    );
    second = launch();
    const secondStarted = await wait(() =>
      second.events.find((event) => event.status === "building"),
    );
    await wait(() => second.child.exitCode !== null, 25000);
    assert.equal(second.child.exitCode, 1);
    assert.notEqual(secondStarted.artifactDirectory, started.artifactDirectory);
    assert.equal(first.child.exitCode, null);
    assert.equal(
      await readFile(
        join(started.artifactDirectory, "native-state.json"),
        "utf8",
      ),
      state,
    );
    await assertClean(secondStarted);
    await stop(first);
    assert.equal(first.child.exitCode, 0);
    await assertClean(started);
  } finally {
    if (second) await stop(second);
    await stop(first);
  }
});
