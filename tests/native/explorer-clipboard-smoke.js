(async () => {
  const directory = SMOKE_DIRECTORY;
  let projectRoot = `${directory}/project with spaces`;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const events = [];
  const focusTrace = [];
  let checkpoint = "startup";
  let clipboardChangeCount = null;
  const startupModalDiagnostics = { before: null, after: null };
  const inspectStartupModal = () => {
    const dialog = document.querySelector(
      "dialog.agent-control-startup-dialog",
    );
    return {
      present: Boolean(dialog),
      open: dialog instanceof HTMLDialogElement ? dialog.open : false,
      title: dialog?.querySelector("h2")?.textContent?.trim() ?? null,
      buttonLabels: dialog
        ? [...dialog.querySelectorAll("button")].map(
            (button) => button.textContent?.trim() || button.title,
          )
        : [],
    };
  };
  const wait = async (check, label = checkpoint) => {
    for (let i = 0; i < 300; i++) {
      const value = await check();
      if (value) return value;
      await pause(100);
    }
    throw Error(`Timed out: ${label}`);
  };

  for (const type of ["copy", "cut", "paste"]) {
    document.addEventListener(
      type,
      (event) => {
        const target = event.target instanceof Element ? event.target : null;
        const entry = target?.closest(".tree-entry");
        const record = {
          type,
          targetTitle: entry?.getAttribute("title") ?? null,
          targetTag: target?.tagName ?? null,
          trusted: event.isTrusted,
          defaultPrevented: null,
        };
        events.push(record);
        setTimeout(() => {
          record.defaultPrevented = event.defaultPrevented;
        }, 0);
      },
      true,
    );
  }
  for (const type of ["focusin", "focusout"]) {
    document.addEventListener(
      type,
      (event) => {
        const target = event.target instanceof Element ? event.target : null;
        focusTrace.push({
          type,
          tag: target?.tagName ?? null,
          className: target?.className?.toString() ?? null,
          title: target?.getAttribute("title") ?? null,
          at: Date.now(),
        });
        if (focusTrace.length > 60) focusTrace.shift();
      },
      true,
    );
  }

  const findEntry = (path) =>
    [...document.querySelectorAll(".tree-entry")].find(
      (entry) => entry.getAttribute("title") === path,
    );
  const selectEntry = async (path) => {
    await wait(() => findEntry(path), `Explorer row ${path}`);
    const entry = findEntry(path);
    if (
      document.activeElement !== entry &&
      document.activeElement instanceof HTMLElement
    )
      document.activeElement.blur();
    entry.focus({ preventScroll: true });
    if (entry.getAttribute("aria-pressed") !== "true")
      entry.dispatchEvent(
        new MouseEvent("click", {
          bubbles: true,
          cancelable: true,
          detail: 1,
          button: 0,
          view: window,
        }),
      );
    await wait(
      () => entry.getAttribute("aria-pressed") === "true",
      `select Explorer row ${path}`,
    );
    const selectedEntry = findEntry(path);
    selectedEntry.focus({ preventScroll: true });
    await wait(
      () =>
        document.activeElement === selectedEntry &&
        selectedEntry.getAttribute("aria-pressed") === "true",
      `select Explorer row ${path}`,
    );
    return selectedEntry;
  };
  const listNames = async (relative) =>
    (await invoke("list_directory", { root: projectRoot, relative })).map(
      (entry) => entry.name,
    );
  const inspectClipboard = async () => {
    const status = await invoke("plugin_smoke_result", {
      stage: "clipboard-status",
      data: null,
    });
    clipboardChangeCount = status.changeCount;
    return status;
  };
  const performEditAction = async (action, expectedPath) => {
    const before = await inspectClipboard();
    if (!before.markerOwned)
      throw Error(`System clipboard changed before AppKit ${action}`);
    const previousLength = events.length;
    await invoke("plugin_smoke_result", {
      stage: "clipboard-menu",
      data: { action },
    });
    await wait(
      () =>
        events
          .slice(previousLength)
          .some(
            (event) => event.type === action && event.defaultPrevented !== null,
          ),
      `native Edit menu ${action} event`,
    );
    const event = events
      .slice(previousLength)
      .find((candidate) => candidate.type === action);
    const after = await inspectClipboard();
    if (!event.trusted)
      throw Error(`AppKit ${action} did not produce a trusted DOM event`);
    if (event.targetTitle !== expectedPath)
      throw Error(
        `AppKit ${action} targeted ${String(event.targetTitle)} instead of ${expectedPath}`,
      );
    if (!event.defaultPrevented)
      throw Error(`Explorer did not prevent the native ${action} default`);
    if (!after.markerOwned)
      throw Error(`The system clipboard marker changed during ${action}`);
    return event;
  };

  try {
    await wait(() => document.querySelector(".file-tree"), "Explorer tree");
    await wait(
      () => document.querySelector("dialog.agent-control-startup-dialog[open]"),
      "AgentControlStartup modal",
    );
    startupModalDiagnostics.before = inspectStartupModal();
    const startupDialog = document.querySelector(
      "dialog.agent-control-startup-dialog[open]",
    );
    if (startupDialog) {
      const closeButton = startupDialog.querySelector(
        'button[title="Close dialog"]',
      );
      if (!closeButton)
        throw Error("AgentControlStartup modal has no Close dialog button");
      closeButton.click();
      await wait(
        () =>
          !document.querySelector("dialog.agent-control-startup-dialog[open]"),
        "dismiss AgentControlStartup modal",
      );
    }
    await pause(300);
    startupModalDiagnostics.after = inspectStartupModal();
    const sourceAtStartup = await wait(
      () =>
        [...document.querySelectorAll(".tree-entry")].find((entry) =>
          entry.getAttribute("title")?.endsWith("/alpha.txt"),
        ),
      "source file row",
    );
    projectRoot = sourceAtStartup
      .getAttribute("title")
      .slice(0, -"/alpha.txt".length);
    await inspectClipboard();
    await invoke("plugin_smoke_result", {
      stage: "focus-window",
      data: null,
    });
    await wait(async () => {
      if (!document.hasFocus()) return false;
      await pause(300);
      return document.hasFocus();
    }, "isolated app window activation");

    checkpoint = "AppKit Edit > Copy on the focused source file row";
    const source = await selectEntry(`${projectRoot}/alpha.txt`);
    await performEditAction("copy", `${projectRoot}/alpha.txt`);
    if (source.closest(".tree-row").getAttribute("data-cut") === "true")
      throw Error("Copy marked the source row as cut");

    checkpoint = "AppKit Edit > Paste into the copy destination folder";
    await selectEntry(`${projectRoot}/copy-dest`);
    await performEditAction("paste", `${projectRoot}/copy-dest`);
    await wait(
      async () => (await listNames("copy-dest")).includes("alpha.txt"),
      "copied file on disk",
    );
    if (!(await listNames("")).includes("alpha.txt"))
      throw Error("Copy removed the source file");

    checkpoint = "AppKit Edit > Cut on the focused source file row";
    const sourceAgain = await selectEntry(`${projectRoot}/alpha.txt`);
    await performEditAction("cut", `${projectRoot}/alpha.txt`);
    if (sourceAgain.closest(".tree-row").getAttribute("data-cut") !== "true")
      throw Error("Cut did not mark the source row");

    checkpoint = "AppKit Edit > Paste into the move destination folder";
    const moveTarget = await selectEntry(`${projectRoot}/move-dest`);
    await performEditAction("paste", `${projectRoot}/move-dest`);
    await wait(
      async () => (await listNames("move-dest")).includes("alpha.txt"),
      "moved file on disk",
    );
    if ((await listNames("")).includes("alpha.txt"))
      throw Error("Cut paste left the source file in its original folder");

    checkpoint = "Explorer refresh after native Cut and Paste";
    await wait(
      () => !findEntry(`${projectRoot}/alpha.txt`),
      "source row removed after move",
    );
    const expand = [...document.querySelectorAll("button")].find(
      (button) => button.getAttribute("aria-label") === "Expand move-dest",
    );
    if (expand) expand.click();
    await wait(
      () => findEntry(`${projectRoot}/move-dest/alpha.txt`),
      "moved file row in Explorer",
    );
    if (moveTarget.getAttribute("aria-expanded") !== "true")
      throw Error("The move target folder did not expand");

    await inspectClipboard();
    await invoke("plugin_smoke_result", {
      stage: "passed",
      data: {
        clipboardEvents: events,
        startupModal: startupModalDiagnostics,
        sourceExistsAfterCopy: true,
        copiedFileExists: true,
        sourceExistsAfterMove: false,
        movedFileExists: true,
        clipboardChangeCount,
      },
    });
  } catch (error) {
    try {
      const status = await inspectClipboard();
      let nativeWindowStatus;
      try {
        nativeWindowStatus = await invoke("plugin_smoke_result", {
          stage: "window-status",
          data: null,
        });
      } catch (windowError) {
        nativeWindowStatus = String(windowError);
      }
      let rootEntries;
      try {
        rootEntries = await listNames("");
      } catch (entryError) {
        rootEntries = String(entryError);
      }
      await invoke("plugin_smoke_result", {
        stage: "failed",
        data: {
          checkpoint,
          error: String(error),
          clipboardEvents: events,
          clipboardStatus: status,
          clipboardChangeCount,
          pageTitle: document.title,
          projectSwitcher:
            document.querySelector(".project-switcher")?.textContent ?? null,
          activeElement: {
            tag: document.activeElement?.tagName ?? null,
            className:
              document.activeElement instanceof Element
                ? (document.activeElement.className?.toString() ?? null)
                : null,
            title:
              document.activeElement instanceof Element
                ? document.activeElement.getAttribute("title")
                : null,
            pressed:
              document.activeElement instanceof Element
                ? document.activeElement.getAttribute("aria-pressed")
                : null,
          },
          documentHasFocus: document.hasFocus(),
          visibilityState: document.visibilityState,
          nativeWindowStatus,
          startupModal: startupModalDiagnostics,
          focusTrace: focusTrace.slice(-30),
          treeEntries: [...document.querySelectorAll(".tree-entry")].map(
            (entry) => ({
              title: entry.getAttribute("title"),
              text: entry.textContent?.trim(),
              pressed: entry.getAttribute("aria-pressed"),
            }),
          ),
          rootEntries,
        },
      });
    } catch (reportError) {
      console.error(
        "Could not report Explorer clipboard smoke failure",
        reportError,
      );
    }
  }
})();
