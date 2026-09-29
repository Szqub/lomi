(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const wait = async (condition) => {
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      if (await condition()) return;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    throw Error(
      "Native agent assertion timed out: " +
        document.body.textContent.slice(-2000),
    );
  };
  let phase = "initial terminal";
  try {
    await wait(() => document.querySelector(".xterm"));
    phase = "installed-only dialog";
    document.querySelector('[aria-label^="New tab"]').click();
    await wait(() =>
      document.querySelector('[role="menuitem"][aria-label="Agents"]'),
    );
    document.querySelector('[role="menuitem"][aria-label="Agents"]').click();
    await wait(() =>
      document.querySelector('.agents-dialog input[type="radio"]'),
    );
    const radios = [
      ...document.querySelectorAll('.agents-dialog input[type="radio"]'),
    ];
    if (radios.length !== 1 || radios[0].value !== "cursor")
      throw Error("Expected only the installed Cursor fixture.");
    if (
      document.querySelector('.agents-dialog input[type="number"]').value !==
      "4"
    )
      throw Error("Expected default count 4.");
    phase = "four immediate native PTYs and background stream acknowledgements";
    document.querySelector('.agents-dialog button[type="submit"]').click();
    await wait(async () => {
      const state = await invoke("plugin_smoke_result", {
        stage: "inspect",
        data: null,
      });
      return state.streamed.trim().split("\n").length === 4;
    });
    await wait(() => !document.querySelector(".agents-dialog"));
    const before = await invoke("plugin_smoke_result", {
      stage: "inspect",
      data: null,
    });
    const launches = before.launches.trim().split("\n");
    if (
      launches.length !== 4 ||
      new Set(launches.map((line) => line.split("\t")[0])).size !== 4
    )
      throw Error("Expected four unique fixture processes.");
    if (launches.some((line) => !line.endsWith("/project")))
      throw Error("Agent cwd changed.");
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    let agentTabs;
    await wait(async () => {
      const saved = await invoke("load_session");
      agentTabs = saved.projects[0].workspaces[0].tabs.filter((tab) =>
        tab.title.startsWith("Cursor CLI"),
      );
      return agentTabs.length === 4;
    });
    const runtimes = agentTabs.map((tab) => runningTerminal(tab.layout.id));
    await wait(
      () => runtimes.filter((runtime) => !runtime.opened).length === 3,
    );
    await wait(() =>
      runtimes.every(
        (runtime) =>
          runtime.getSnapshot().title ===
          `Agent fixture ${before.terminals[runtime.sessionId]}`,
      ),
    );
    phase = "native tab switch continuity";
    const tabs = [...document.querySelectorAll('[role="tab"]')].filter((tab) =>
      tab.textContent.includes("Cursor CLI"),
    );
    if (tabs.length !== 4) throw Error("Expected four Cursor tabs.");
    const titles = new Set();
    for (const [index, tab] of tabs.entries()) {
      tab.click();
      const expectedTitle = `Agent fixture ${before.terminals[runtimes[index].sessionId]}`;
      await wait(
        () =>
          tab.getAttribute("aria-selected") === "true" &&
          document.querySelector(".terminal-title")?.textContent ===
            expectedTitle,
      );
      titles.add(document.querySelector(".terminal-title").textContent);
    }
    if (titles.size !== 4)
      throw Error("Expected four distinct retained titles.");
    const after = await invoke("plugin_smoke_result", {
      stage: "inspect",
      data: null,
    });
    if (
      JSON.stringify(after.terminals) !== JSON.stringify(before.terminals) ||
      after.launches !== before.launches
    )
      throw Error("Switching tabs restarted an agent.");
    phase = "settings caller denial";
    await invoke("plugin_smoke_result", {
      stage: "check-settings",
      data: {
        checks: [
          "installed-only native discovery",
          "four immediate PTYs in project cwd",
          "hidden output flow control",
          "background title parsing before attachment",
          "tab switches retain processes",
        ],
        launches: 4,
      },
    });
  } catch (error) {
    await invoke("plugin_smoke_result", {
      stage: "failed",
      data: { phase, error: String(error) },
    });
  }
})();
