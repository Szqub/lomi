import assert from "node:assert/strict";
import { test } from "node:test";
import {
  layoutFits,
  layoutPositions,
  maximumAgentCount,
  minimumLayoutSize,
  MIN_PANE_HEIGHT,
  MIN_PANE_WIDTH,
  newTab,
  panes,
  SPLIT_DIVIDER_SIZE,
} from "../src/model.ts";
import type { LayoutSize } from "../src/model.ts";

const gridSize = (columns: number, rows: number): LayoutSize => ({
  width: columns * MIN_PANE_WIDTH + (columns - 1) * SPLIT_DIVIDER_SIZE,
  height: rows * MIN_PANE_HEIGHT + (rows - 1) * SPLIT_DIVIDER_SIZE,
});

test("agent capacity includes the minimum pane size and each divider", () => {
  assert.equal(maximumAgentCount(gridSize(1, 1)), 1);
  const size = gridSize(4, 3);
  assert.equal(maximumAgentCount(size), 12);
  assert.equal(maximumAgentCount({ ...size, width: size.width - 0.1 }), 9);
  assert.equal(maximumAgentCount({ ...size, height: size.height - 0.1 }), 8);
  assert.equal(maximumAgentCount({ width: 1000, height: 250 }), 8);
  assert.equal(maximumAgentCount(gridSize(20, 10)), 200);
});

test("missing, invalid, and insufficient terminal space has zero capacity", () => {
  const invalid = [
    undefined,
    null,
    {},
    { width: 1000 },
    { height: 1000 },
    { width: MIN_PANE_WIDTH - 0.1, height: MIN_PANE_HEIGHT },
    { width: MIN_PANE_WIDTH, height: MIN_PANE_HEIGHT - 0.1 },
    ...[0, -1, NaN, Infinity, -Infinity].flatMap((dimension) => [
      { width: dimension, height: 1000 },
      { width: 1000, height: dimension },
    ]),
  ];
  for (const size of invalid)
    assert.equal(maximumAgentCount(size as LayoutSize), 0);
});

test("size-aware tabs reject excess counts before allocating identities", (t) => {
  t.mock.method(crypto, "randomUUID", () => {
    throw new Error("Unexpected pane allocation");
  });
  for (const count of [9, Number.MAX_SAFE_INTEGER])
    assert.throws(
      () =>
        newTab("/project", "local:bash", "Agents", count, {
          width: 1000,
          height: 250,
        }),
      /do not fit/,
    );
  assert.throws(
    () => newTab("/project", "local:bash", "Agents", 1, gridSize(0, 0)),
    /do not fit/,
  );
});

test("size-aware tabs adapt to wide-short and tall-narrow terminal areas", () => {
  for (const [size, count, expected] of [
    [{ width: 1000, height: 250 }, 8, gridSize(4, 2)],
    [gridSize(1, 8), 8, gridSize(1, 8)],
    [gridSize(2, 4), 8, gridSize(2, 4)],
  ] as const) {
    const tab = newTab("/project", "local:bash", "Agents", count, size);
    assert.equal(panes(tab.layout).length, count);
    assert.deepEqual(minimumLayoutSize(tab.layout), expected);
    assert.ok(layoutFits(tab.layout, size));
  }
});

test("every count within capacity fits, including unequal rows", () => {
  for (let columns = 1; columns <= 6; columns++) {
    for (let rows = 1; rows <= 6; rows++) {
      const size = gridSize(columns, rows);
      for (let count = 1; count <= maximumAgentCount(size); count++) {
        const tab = newTab("/project", "local:bash", "Agents", count, size);
        assert.ok(layoutFits(tab.layout, size));
        for (const { layout, bounds } of layoutPositions(tab.layout, size)) {
          if (layout.type === "split") continue;
          assert.ok(bounds.width >= MIN_PANE_WIDTH - 1e-9);
          assert.ok(bounds.height >= MIN_PANE_HEIGHT - 1e-9);
        }
      }
    }
  }
});

test("tabs preserve their preferred layout with ample space or no size", () => {
  for (const count of [1, 2, 3, 4, 5, 6, 8, 9, 12, 100]) {
    const size = gridSize(12, 12);
    const legacy = newTab("/project", "local:bash", "Agents", count);
    const sized = newTab("/project", "local:bash", "Agents", count, size);
    const positions = (layout: typeof legacy.layout) =>
      layoutPositions(layout, size).map(({ layout, bounds }) => ({
        type: layout.type,
        bounds,
        ratio: layout.type === "split" ? layout.ratio : undefined,
      }));
    assert.deepEqual(positions(sized.layout), positions(legacy.layout));
  }
  assert.deepEqual(
    minimumLayoutSize(newTab("/project", "local:bash", "Agents", 8).layout),
    gridSize(3, 3),
  );
});
