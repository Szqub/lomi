import { expect, test, type Page } from "@playwright/test";
import { mockDesktop } from "./desktop";

test.use({ colorScheme: "dark", reducedMotion: "reduce" });

// Fixed hosts avoid a split resize rebuilding the existing pane's render model
// and accidentally hiding stale texture coordinates. Both use the real runtime.
async function mountTerminal(page: Page, name: string, left: number) {
  await page.evaluate(
    async ({ name, left }) => {
      const { TerminalRuntime, runningTerminal } =
        await import("/src/terminal-runtime.ts");
      const pane = document.querySelector<HTMLElement>("[data-pane-id]")!;
      const profile = runningTerminal(pane.dataset.paneId!)!.profile;
      const host = document.createElement("div");
      host.id = name;
      Object.assign(host.style, {
        position: "fixed",
        left: `${left}px`,
        top: "100px",
        width: "620px",
        height: "220px",
        zIndex: "9999",
        background: "#101010",
      });
      document.body.append(host);
      const runtime = new TerminalRuntime(name, profile, "/project");
      runtime.terminal.options.cursorBlink = false;
      (window as any).__atlasTerminals ??= {};
      (window as any).__atlasTerminals[name] = runtime;
      runtime.attach(host);
      await (runtime as any).rendererPromise;
      await (runtime as any).startPromise;
    },
    { name, left },
  );
  await expect(page.locator(`#${name} .terminal-host`)).toHaveCSS(
    "opacity",
    "1",
  );
  expect(
    await page.evaluate(
      (name) => (window as any).__atlasTerminals[name].getSnapshot().renderer,
      name,
    ),
  ).toBe("WebGL");
}

async function write(page: Page, name: string, data: string) {
  await page.evaluate(
    async ({ name, data }) => {
      const terminal = (window as any).__atlasTerminals[name].terminal;
      await new Promise<void>((resolve) => terminal.write(data, resolve));
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      );
    },
    { name, data },
  );
}

async function terminalState(page: Page) {
  return page.evaluate(() => {
    const runtime = (window as any).__atlasTerminals["atlas-first"];
    const terminal = runtime.terminal;
    return {
      sessionId: runtime.sessionId,
      running: (window as any).__nativeTest.sessions.has(runtime.sessionId),
      starts: (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "start_terminal" &&
          call.args.request.id === runtime.sessionId,
      ).length,
      closes: (window as any).__nativeTest.calls.filter(
        (call: any) =>
          call.command === "close_terminal" &&
          call.args.id === runtime.sessionId,
      ).length,
      cols: terminal.cols,
      rows: terminal.rows,
      viewport: terminal.buffer.active.viewportY,
      content: Array.from({ length: terminal.rows }, (_, row) =>
        terminal.buffer.active.getLine(row)?.translateToString(true),
      ),
      selection: terminal.getSelection(),
    };
  });
}

async function changedPixels(page: Page, before: Buffer, after: Buffer) {
  return page.evaluate(
    async ({ before, after }) => {
      const images = await Promise.all(
        [before, after].map(async (png) => {
          const image = new Image();
          image.src = `data:image/png;base64,${png}`;
          await image.decode();
          const canvas = document.createElement("canvas");
          canvas.width = image.width;
          canvas.height = image.height;
          const context = canvas.getContext("2d")!;
          context.drawImage(image, 0, 0);
          return {
            width: image.width,
            height: image.height,
            pixels: context.getImageData(0, 0, image.width, image.height).data,
          };
        }),
      );
      if (
        images[0].width !== images[1].width ||
        images[0].height !== images[1].height
      )
        throw new Error("Terminal screenshot geometry changed");
      let changed = 0;
      for (let i = 0; i < images[0].pixels.length; i += 4)
        if (
          images[0].pixels
            .slice(i, i + 4)
            .some((value, channel) => value !== images[1].pixels[i + channel])
        )
          changed++;
      return changed;
    },
    { before: before.toString("base64"), after: after.toString("base64") },
  );
}

for (const invalidate of ["sibling startup", "explicit shared atlas clear"]) {
  test(`preserves painted glyphs after ${invalidate}`, async ({
    page,
  }, testInfo) => {
    await mockDesktop(page, false);
    await page.goto("/");
    await expect(page.locator(".terminal-host")).toHaveCSS("opacity", "1");
    await mountTerminal(page, "atlas-first", 20);
    if (invalidate === "explicit shared atlas clear")
      await mountTerminal(page, "atlas-second", 680);

    // Non-ASCII and styled glyphs escape the atlas's default ASCII warmup.
    const content =
      "\x1b[?25l\x1b[2J\x1b[HΩЖλ▓╬●→éøñ\r\n" +
      "\x1b[1;38;2;240;190;80mABCDEFGHIJKLM\x1b[0m\x1b[6;1H.";
    await write(page, "atlas-first", content);
    await page.evaluate(() =>
      (window as any).__atlasTerminals["atlas-first"].terminal.select(0, 0, 4),
    );
    const screen = page.locator("#atlas-first .xterm-screen");
    const beforeState = await terminalState(page);
    expect(beforeState.running).toBe(true);
    expect(beforeState.starts).toBe(1);
    expect(beforeState.closes).toBe(0);
    expect(beforeState.selection).toBe("ΩЖλ▓");
    const before = await screen.screenshot({
      path: testInfo.outputPath("atlas-before.png"),
    });
    await page.evaluate(() => {
      const renderer = (window as any).__atlasTerminals["atlas-first"].webgl
        ._renderer;
      const clear = renderer._clearModel.bind(renderer);
      (window as any).__atlasModelClears = 0;
      renderer._clearModel = (...args: any[]) => {
        (window as any).__atlasModelClears++;
        return clear(...args);
      };
    });

    if (invalidate === "sibling startup")
      await mountTerminal(page, "atlas-second", 680);
    expect(
      await page.evaluate(() => {
        const terminals = (window as any).__atlasTerminals;
        const first = terminals["atlas-first"].webgl._renderer;
        const second = terminals["atlas-second"].webgl._renderer;
        return first._charAtlas === second._charAtlas;
      }),
    ).toBe(true);
    if (invalidate === "explicit shared atlas clear")
      await page.evaluate(() =>
        (window as any).__atlasTerminals[
          "atlas-second"
        ].terminal.clearTextureAtlas(),
      );
    // Repack the shared texture in a different order, then dirty just one row
    // of the original terminal without changing its parsed content.
    await write(page, "atlas-second", "\x1b[?25l\x1b[2J\x1b[Hñøé→●╬▓λЖΩ");
    await write(page, "atlas-first", "\x1b[6;1H.");
    expect(await terminalState(page)).toEqual(beforeState);
    const after = await screen.screenshot({
      path: testInfo.outputPath("atlas-after.png"),
    });
    expect(
      await changedPixels(page, before, after),
      "Unchanged terminal text must keep identical painted pixels after shared atlas invalidation",
    ).toBe(0);
    expect(await page.evaluate(() => (window as any).__atlasModelClears)).toBe(
      1,
    );
    // A later partial refresh must reuse the rebuilt model, rather than rebuild
    // all rows on every frame after the shared clear.
    await write(page, "atlas-first", "\x1b[6;1H.");
    expect(await page.evaluate(() => (window as any).__atlasModelClears)).toBe(
      1,
    );
  });
}
