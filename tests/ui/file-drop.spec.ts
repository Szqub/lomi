import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";
import { newPane, newProject, newSession, splitPane } from "../../src/model";
import { buffer, mockDesktop } from "./desktop";

type Platform = "macos" | "linux" | "windows";

async function setup(
  page: Page,
  platform: Platform,
  scale = 2,
  zoom = 1,
  layout: "horizontal" | "vertical" | "nested" = "nested",
) {
  const project = newProject("/project", "local:bash");
  const tab = project.workspaces[0].tabs[0];
  if (tab.type !== "terminal") throw new Error("Expected terminal tab");
  const first = tab.activePaneId;
  const second = newPane("/project/second");
  const third = newPane("/project/third");
  tab.layout =
    layout === "nested"
      ? splitPane(
          splitPane(tab.layout, first, "horizontal", second),
          second.id,
          "vertical",
          third,
        )
      : splitPane(tab.layout, first, layout, second);
  await mockDesktop(
    page,
    false,
    { ...newSession(), projects: [project], activeProjectId: project.id },
    undefined,
    {},
    platform,
  );
  await page.addInitScript(
    ({ platform, scale, zoom }) => {
      localStorage.setItem("lomi.zoom.main", String(zoom * 100));
      Object.defineProperty(window, "devicePixelRatio", {
        configurable: true,
        value: platform === "windows" ? scale * zoom : scale,
      });
    },
    { platform, scale, zoom },
  );
  await page.goto("/");
  const ids =
    layout === "nested" ? [first, second.id, third.id] : [first, second.id];
  await expect(page.locator(".xterm-screen")).toHaveCount(ids.length);
  for (const id of ids)
    await expect.poll(() => buffer(page, id)).toContain("bash $ ");
  await expect
    .poll(() => page.evaluate(() => (window as any).__nativeTest.zoom))
    .toBe(zoom);
  return ids;
}

async function drag(
  page: Page,
  type: "enter" | "over" | "drop" | "leave",
  target: Locator,
  factor = 1,
  paths = ["/external/it's a file żółć.txt", "/external/second.png"],
) {
  const bounds = (await target.boundingBox())!;
  await page.evaluate(
    ({ type, position, paths }) =>
      (window as any).__nativeTest.emitEvent(`tauri://drag-${type}`, {
        position,
        paths,
      }),
    {
      type,
      position: {
        x: (bounds.x + bounds.width / 2) * factor,
        y: (bounds.y + bounds.height / 2) * factor,
      },
      paths,
    },
  );
}

const writes = (page: Page) =>
  page.evaluate(() =>
    (window as any).__nativeTest.calls
      .filter((call: any) => call.command === "write_terminal")
      .map((call: any) => call.args),
  );

const terminalSession = (page: Page, id: string) =>
  page.evaluate(async (id) => {
    const { runningTerminal } = await import("/src/terminal-runtime.ts");
    return runningTerminal(id)!.sessionId;
  }, id);

for (const [platform, scale, zoom] of [
  ["macos", 1, 1],
  ["macos", 2, 1],
  ["macos", 2, 0.8],
  ["macos", 2, 1.4],
  ["linux", 2, 1],
  ["linux", 2, 1.4],
  ["windows", 1, 1],
  ["windows", 2, 1.4],
] as const) {
  test(`${platform} file drops follow all three panes at scale ${scale} and zoom ${zoom}`, async ({
    page,
  }, testInfo) => {
    const ids = await setup(page, platform, scale, zoom);
    const factor = platform === "windows" ? scale * zoom : zoom;
    const pane = (id: string) => page.locator(`[data-pane-id="${id}"]`);
    for (const [index, id] of ids.entries()) {
      const other = pane(ids[(index + 1) % ids.length]);
      await other.locator(".xterm-helper-textarea").focus();
      await drag(page, "enter", other, factor);
      await expect(other).toHaveClass(/drop-target/);
      await drag(page, "over", pane(id), factor);
      await expect(pane(id)).toHaveClass(/drop-target/);
      await expect(page.locator(".drop-target")).toHaveCount(1);
      await expect(other.locator(".xterm-helper-textarea")).toBeFocused();
      if (platform === "macos" && scale === 2 && zoom === 1 && index === 2)
        await page.screenshot({ path: testInfo.outputPath("drop-target.png") });
      await drag(page, "drop", pane(id), factor);
      await expect
        .poll(async () => (await writes(page)).length)
        .toBe(index + 1);
      const sent = (await writes(page))[index];
      expect(sent.id).toBe(await terminalSession(page, id));
      expect(sent.data).toBe(
        "'/external/it'\\''s a file żółć.txt' '/external/second.png' ",
      );
      await expect(pane(id).locator(".xterm-helper-textarea")).toBeFocused();
      await expect(page.locator(".drop-target")).toHaveCount(0);
    }
    const calls = await page.evaluate(() => (window as any).__nativeTest.calls);
    expect(
      calls.filter((call: any) => call.command === "start_terminal"),
    ).toHaveLength(3);
    expect(
      calls.filter((call: any) => call.command === "close_terminal"),
    ).toHaveLength(0);
  });
}

test("leaving or dropping outside terminals clears the target without pasting", async ({
  page,
}) => {
  const ids = await setup(page, "macos");
  const terminal = page.locator(`[data-pane-id="${ids[2]}"]`);
  await drag(page, "enter", terminal);
  await expect(terminal).toHaveClass(/drop-target/);
  await drag(page, "leave", terminal);
  await expect(page.locator(".drop-target")).toHaveCount(0);
  await drag(page, "enter", terminal);
  await drag(page, "drop", page.locator(".file-tree"));
  await expect(page.locator(".drop-target")).toHaveCount(0);
  expect(await writes(page)).toEqual([]);
  await drag(page, "enter", terminal);
  const final = page.locator(`[data-pane-id="${ids[1]}"]`);
  await drag(page, "drop", final);
  await expect
    .poll(async () => (await writes(page)).map((write: any) => write.id))
    .toEqual([await terminalSession(page, ids[1])]);
});

test("Explorer pointer dragging reaches the third terminal on Retina", async ({
  page,
}) => {
  const ids = await setup(page, "macos");
  const file = page.getByRole("button", {
    name: "it's a file.txt",
    exact: true,
  });
  const terminal = page.locator(`[data-pane-id="${ids[2]}"]`);
  const bounds = (await terminal.boundingBox())!;
  await file.hover();
  await page.mouse.down();
  await page.mouse.move(
    bounds.x + bounds.width / 2,
    bounds.y + bounds.height / 2,
    { steps: 8 },
  );
  await expect(terminal).toHaveClass(/drop-target/);
  await page.mouse.up();
  await expect
    .poll(async () => (await writes(page)).map((write: any) => write.id))
    .toEqual([await terminalSession(page, ids[2])]);
  await expect(terminal.locator(".xterm-helper-textarea")).toBeFocused();
});

async function paintedBorder(page: Page, target: Locator) {
  const bounds = (await target.boundingBox())!;
  const color = await target.evaluate(
    (element) => getComputedStyle(element, "::after").borderTopColor,
  );
  const png = await target.screenshot();
  return page.evaluate(
    async ({ png, bounds, color }) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const context = canvas.getContext("2d")!;
      context.drawImage(image, 0, 0);
      const pixels = context.getImageData(0, 0, image.width, image.height).data;
      const rgb = color
        .match(/[\d.]+/g)!
        .slice(0, 3)
        .map(Number);
      const scaleX = image.width / bounds.width;
      const scaleY = image.height / bounds.height;
      // Search a narrow band along each edge, excluding corners and the centered
      // overlay label. Count painted dash positions rather than configured borders.
      const edges = ["left", "top", "right", "bottom"] as const;
      return {
        scaleX,
        scaleY,
        coverage: Object.fromEntries(
          edges.map((edge) => {
            const vertical = edge === "left" || edge === "right";
            const length = vertical ? image.height : image.width;
            const depth = Math.ceil(3 * (vertical ? scaleX : scaleY));
            const start = Math.ceil(length * 0.2);
            const end = Math.floor(length * 0.8);
            let painted = 0;
            for (let along = start; along < end; along++) {
              for (let across = 0; across < depth; across++) {
                const x = vertical
                  ? edge === "left"
                    ? across
                    : image.width - 1 - across
                  : along;
                const y = vertical
                  ? along
                  : edge === "top"
                    ? across
                    : image.height - 1 - across;
                const index = (y * image.width + x) * 4;
                if (
                  rgb.every(
                    (value, channel) =>
                      Math.abs(pixels[index + channel] - value) < 30,
                  )
                ) {
                  painted++;
                  break;
                }
              }
            }
            return [edge, painted / (end - start)];
          }),
        ),
      };
    },
    { png: png.toString("base64"), bounds, color },
  );
}

test.describe("file-drop borders at Retina resolution", () => {
  test.use({ deviceScaleFactor: 2 });
  for (const colorScheme of ["light", "dark"] as const) {
    for (const layout of ["horizontal", "vertical", "nested"] as const) {
      test(`${colorScheme} paints every target edge in ${layout} splits`, async ({
        page,
      }, testInfo) => {
        await page.emulateMedia({ colorScheme });
        const ids = await setup(page, "macos", 2, 1, layout);
        for (const [index, id] of ids.entries()) {
          const target = page.locator(`[data-pane-id="${id}"]`);
          await drag(page, index ? "over" : "enter", target);
          await expect(target).toHaveClass(/drop-target/);
          const border = await paintedBorder(page, target);
          expect(border.scaleX).toBeGreaterThan(1.9);
          expect(border.scaleY).toBeGreaterThan(1.9);
          for (const [edge, coverage] of Object.entries(border.coverage)) {
            expect
              .soft(
                coverage,
                `${layout} pane ${index + 1} ${edge} dashed border coverage`,
              )
              .toBeGreaterThan(0.2);
          }
          if (index === ids.length - 1)
            await page.screenshot({
              path: testInfo.outputPath(
                `drop-border-${layout}-${colorScheme}.png`,
              ),
            });
        }
        await drag(
          page,
          "leave",
          page.locator(`[data-pane-id="${ids.at(-1)}"]`),
        );
        await expect(page.locator(".drop-target")).toHaveCount(0);
        expect(await writes(page)).toEqual([]);
      });
    }
  }
});
