import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

const beforeHash = "a".repeat(64);
const content = "# Project\nA text file preview.\n";
async function setup(page: Page, readOnly = false) {
  await mockDesktop(page, true, undefined, undefined, {
    "/project/README.md": {
      content,
      revision: beforeHash,
      encoding: "utf8",
      readOnly,
    },
  });
  await page.goto("/");
}
async function openReadme(page: Page) {
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await expect(page.locator(".cm-content")).toBeVisible();
}

test("coding approval rejects actual dirty text and releases its opening gate", async ({
  page,
}) => {
  await setup(page);
  await openReadme(page);
  await page.locator(".cm-content").focus();
  await page.keyboard.insertText("unsaved");
  const result = await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const doc = runtime.documents()[0];
    let failure = "accepted";
    try {
      await runtime.freezeCodingEffect("/project/README.md", hash);
    } catch (error) {
      failure = (error as Error).message;
    }
    return { failure, dirty: doc.dirty, frozen: doc.codingEffectFrozen };
  }, beforeHash);
  expect(result).toEqual({
    failure: "TARGET_BUSY",
    dirty: true,
    frozen: false,
  });
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
  await page
    .getByRole("button", { name: "it's a file.txt", exact: true })
    .dblclick();
  await expect(page.locator(".cm-content")).toContainText("Hello, 🦀!");
});

test("coding fence blocks bypass edits and opens until its last idempotent release", async ({
  page,
}) => {
  await setup(page);
  await openReadme(page);
  const result = await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const doc = runtime.documents()[0];
    const first = await runtime.freezeCodingEffect("/project/README.md", hash);
    const second = await runtime.freezeCodingEffect("/project/README.md", hash);
    (window as any).__codingFence = second;
    const failures: string[] = [];
    for (const attempt of [
      () => doc.readAgentBuffer({}),
      () => doc.applyAgentEdits({}, "/project/README.md"),
      () =>
        runtime.openDocument({
          type: "file",
          id: "frozen",
          root: "/project",
          relative: "README.md",
          title: "README.md",
        }),
      () =>
        runtime.openDocument({
          type: "file",
          id: "new",
          root: "/project",
          relative: "new.txt",
          title: "new.txt",
        }),
      () => runtime.freezeCodingEffect("/project/README.md", "b".repeat(64)),
    ]) {
      try {
        await attempt();
        failures.push("accepted");
      } catch (error) {
        failures.push((error as Error).message);
      }
    }
    // This bypasses CodeMirror transaction filters; the dispatch guard must
    // still reject the change, including callers outside the agent API.
    doc.view.dispatch({
      changes: { from: 0, insert: "bypass" },
      filter: false,
    });
    first.release();
    first.release();
    return {
      proof: first.proof,
      failures,
      text: doc.state.doc.toString(),
      frozen: doc.codingEffectFrozen,
    };
  }, beforeHash);
  expect(result.proof.path).toBe("/project/README.md");
  expect(result.proof.documents).toHaveLength(1);
  expect(result.proof.documents[0].diskRevision).toBe(beforeHash);
  expect(result.proof.documents[0].bufferRevision).toContain(
    result.proof.documents[0].documentId,
  );
  expect(result.failures).toEqual([
    "TARGET_BUSY",
    "TARGET_BUSY",
    "TARGET_BUSY",
    "TARGET_BUSY",
    "REVISION_CONFLICT",
  ]);
  expect(result.text).toBe(content);
  expect(result.frozen).toBe(true);
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "false",
  );
  await page.evaluate(() => {
    (window as any).__nativeTest.editorFiles["/project/README.md"] = {
      content: "effect committed\n",
      revision: "b".repeat(64),
      encoding: "utf8",
      readOnly: false,
    };
    (window as any).__codingFence.release();
  });
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
  await expect(page.locator(".cm-content")).toContainText("effect committed");
});

test("coding proof waits for an already opening target and freezes the loaded buffer", async ({
  page,
}) => {
  await setup(page);
  await page.evaluate(() => {
    (window as any).__nativeTest.fileReadDelays["README.md"] = 500;
  });
  await page.getByRole("button", { name: "README.md", exact: true }).dblclick();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).__nativeTest.calls.some(
          (call: any) =>
            call.command === "read_editor_file" &&
            call.args.relative === "README.md",
        ),
      ),
    )
    .toBe(true);
  const proof = await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const fence = await runtime.freezeCodingEffect("/project/README.md", hash);
    (window as any).__codingFence = fence;
    return fence.proof;
  }, beforeHash);
  expect(proof.documents).toHaveLength(1);
  expect(proof.documents[0].diskRevision).toBe(beforeHash);
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "false",
  );
  await page.evaluate(() => (window as any).__codingFence.release());
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "true",
  );
});

test("coding release preserves original disk read-only permissions", async ({
  page,
}) => {
  await setup(page, true);
  await openReadme(page);
  await page.evaluate(async (hash) => {
    const path = "/src/editor-runtime.ts";
    const runtime = await import(path);
    const fence = await runtime.freezeCodingEffect("/project/README.md", hash);
    fence.release();
    fence.release();
  }, beforeHash);
  await expect(page.locator(".cm-content")).toHaveAttribute(
    "contenteditable",
    "false",
  );
});

async function newDraft(page: Page) {
  await page.getByRole("button", { name: /^New tab/ }).click();
  await page.getByRole("menuitem", { name: "New file", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
}

test("a coding fence bars untitled Save As before any native write", async ({
  page,
}) => {
  await setup(page);
  await newDraft(page);
  const result = await page.evaluate(async (hash) => {
    const runtimePath = "/src/editor-runtime.ts";
    const runtime = await import(runtimePath);
    const doc = runtime.documents()[0];
    const fence = await runtime.freezeCodingEffect("/project/README.md", hash);
    let failure = "accepted";
    try {
      await doc.save();
    } catch (error) {
      failure = (error as Error).message;
    }
    const calls = (window as any).__nativeTest.calls.filter(
      (call: any) => call.command === "save_new_editor_file",
    ).length;
    fence.release();
    return { failure, calls };
  }, beforeHash);
  expect(result).toEqual({ failure: "TARGET_BUSY", calls: 0 });
});

test("coding proof drains an earlier Save As and freezes its newly canonical alias", async ({
  page,
}) => {
  await setup(page);
  await newDraft(page);
  await page.evaluate(
    async ({ hash, text }) => {
      const native = (window as any).__nativeTest;
      native.newFilePath = "/project/README.md";
      const internals = (window as any).__TAURI_INTERNALS__;
      const invoke = internals.invoke;
      let finish!: () => void;
      const pending = new Promise<void>((resolve) => {
        finish = resolve;
      });
      (window as any).__finishEarlierSave = finish;
      internals.invoke = async (command: string, args: unknown) => {
        if (command !== "save_new_editor_file") return invoke(command, args);
        (window as any).__earlierSaveRequested = true;
        await pending;
        const result = await invoke(command, args);
        result.file.revision = hash;
        native.editorFiles["/project/README.md"].revision = hash;
        return result;
      };
      const runtimePath = "/src/editor-runtime.ts";
      const runtime = await import(runtimePath);
      const doc = runtime.documents()[0];
      doc.dispatch({ changes: { from: 0, insert: text } });
      (window as any).__earlierSave = doc.save();
      (window as any).__codingFreezeSettled = false;
      (window as any).__codingFreeze = runtime
        .freezeCodingEffect("/project/README.md", hash)
        .then((fence: any) => {
          (window as any).__codingFreezeSettled = true;
          (window as any).__codingFence = fence;
          return fence;
        });
    },
    { hash: beforeHash, text: content },
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).__earlierSaveRequested))
    .toBe(true);
  expect(await page.evaluate(() => (window as any).__codingFreezeSettled)).toBe(
    false,
  );
  const result = await page.evaluate(async () => {
    (window as any).__finishEarlierSave();
    await (window as any).__earlierSave;
    const fence = await (window as any).__codingFreeze;
    const runtimePath = "/src/editor-runtime.ts";
    const runtime = await import(runtimePath);
    const doc = runtime.documents()[0];
    const result = {
      documents: fence.proof.documents.length,
      frozen: doc.codingEffectFrozen,
      revision: fence.proof.documents[0].diskRevision,
    };
    fence.release();
    return result;
  });
  expect(result).toEqual({ documents: 1, frozen: true, revision: beforeHash });
});

test("global file-operation and coding leases exclude each other even with no loaded editor", async ({
  page,
}) => {
  await setup(page);
  const result = await page.evaluate(async (hash) => {
    const runtimePath = "/src/editor-runtime.ts";
    const runtime = await import(runtimePath);
    const servicePath = "/src/editor-service.ts";
    const service = await import(servicePath);
    const failures: string[] = [];
    const pause = await service.pauseEditorFileOperations();
    const paused = service.editorFileOperationsPaused();
    try {
      await runtime.freezeCodingEffect("/project/README.md", hash);
    } catch (error) {
      failures.push((error as Error).message);
    }
    try {
      await service.pauseEditorFileOperations();
    } catch (error) {
      failures.push((error as Error).message);
    }
    pause();
    pause();
    const fence = await runtime.freezeCodingEffect("/project/README.md", hash);
    try {
      await service.pauseEditorFileOperations();
    } catch (error) {
      failures.push((error as Error).message);
    }
    const documents = fence.proof.documents.length;
    fence.release();
    fence.release();
    const resumed = await service.pauseEditorFileOperations();
    resumed();
    return {
      paused,
      documents,
      failures,
      released: !service.editorFileOperationsPaused(),
    };
  }, beforeHash);
  expect(result).toEqual({
    paused: true,
    documents: 0,
    failures: ["TARGET_BUSY", "TARGET_BUSY", "TARGET_BUSY"],
    released: true,
  });
});
