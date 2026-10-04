import assert from "node:assert/strict";
import test from "node:test";
import {
  acceptSnapshot,
  finalRunIds,
  moveAccount,
  quotaLabel,
  isRunning,
  isQuotaFresh,
  hasComparableQuota,
  compatibleGatewayProtocol,
  gatewayProtocols,
  gatewayProtocolsForCli,
} from "../src/router/model.ts";
import type {
  CliRouterSnapshot,
  CliQuota,
  CliProfile,
  CliRouter,
} from "../src/router/types.ts";
const snapshot = (revision: number): CliRouterSnapshot => ({
  revision,
  profiles: [],
  routers: [],
  quota: [],
  runs: [],
  capabilities: [],
});
test("canonical Gateway protocols support every pair while native Codex retains Responses context", () => {
  for (const native of gatewayProtocols) {
    for (const upstream of gatewayProtocols) {
      assert.equal(compatibleGatewayProtocol(native.id, upstream.id), true);
      assert.equal(
        compatibleGatewayProtocol(native.id, upstream.id, "codex"),
        native.id === "openai_responses" && upstream.id === "openai_responses",
      );
      for (const cli of [
        "claude",
        "grok",
        "kimi",
        "kilo",
        "opencode",
        "pi",
        "agy",
      ]) {
        assert.equal(
          compatibleGatewayProtocol(native.id, upstream.id, cli),
          true,
        );
      }
    }
  }
  for (const cli of [undefined, "codex", "claude"]) {
    assert.equal(
      compatibleGatewayProtocol("openai_responses", undefined, cli),
      false,
    );
    assert.equal(
      compatibleGatewayProtocol("unknown", "openai_responses", cli),
      false,
    );
    assert.equal(
      compatibleGatewayProtocol("openai_responses", "unknown", cli),
      false,
    );
  }
});
test("Settings offers only Responses for native Codex and four protocols for the other focused clients", () => {
  assert.deepEqual(
    gatewayProtocolsForCli("codex").map((item) => item.id),
    ["openai_responses"],
  );
  for (const cli of [
    "claude",
    "grok",
    "kimi",
    "kilo",
    "opencode",
    "pi",
    "agy",
  ]) {
    assert.deepEqual(gatewayProtocolsForCli(cli), gatewayProtocols);
  }
  assert.deepEqual(gatewayProtocolsForCli(), gatewayProtocols);
});
test("late router snapshots cannot replace newer configuration or output", () => {
  const current = snapshot(12);
  assert.equal(acceptSnapshot(current, snapshot(11)), current);
  assert.equal(acceptSnapshot(undefined, current), current);
  assert.equal(acceptSnapshot(current, snapshot(13)).revision, 13);
});
test("account ordering moves only the requested account and respects boundaries", () => {
  const ids = ["first", "second", "third"];
  assert.deepEqual(moveAccount(ids, "second", -1), [
    "second",
    "first",
    "third",
  ]);
  assert.deepEqual(moveAccount(ids, "second", 1), ["first", "third", "second"]);
  assert.equal(moveAccount(ids, "first", -1), ids);
  assert.equal(moveAccount(ids, "missing", 1), ids);
  assert.deepEqual(ids, ["first", "second", "third"]);
});
test("quota displays only reported fresh remaining values", () => {
  const quota: CliQuota = {
    profileId: "account",
    status: "fresh",
    windows: [
      { id: "short", remainingPercent: 70, resetAt: null },
      { id: "long", remainingPercent: 20, resetAt: null },
    ],
    observedAt: Date.now() - 1000,
    expiresAt: Date.now() + 20_000,
    epoch: 1,
    blockRevision: 1,
  };
  assert.equal(quotaLabel(quota), "20% remaining");
  assert.equal(quotaLabel({ ...quota, status: "stale" }), "Quota stale");
  assert.equal(
    quotaLabel({
      ...quota,
      windows: [{ id: "unknown", remainingPercent: null, resetAt: null }],
    }),
    "Quota unknown",
  );
  assert.equal(quotaLabel(undefined), "Quota unknown");
});
test("only active execution states poll for output", () => {
  assert.equal(isRunning("running"), true);
  assert.equal(isRunning("starting"), true);
  assert.equal(isRunning("switching"), true);
  assert.equal(isRunning("recovery_required"), false);
  assert.equal(isRunning("completed"), false);
});

const now = 1_000_000;
function report(profileId = "first", epoch = 3): CliQuota {
  return {
    profileId,
    status: "fresh",
    windows: [
      { id: "short", remainingPercent: 80, resetAt: null },
      { id: "long", remainingPercent: 25, resetAt: null },
    ],
    observedAt: now - 1000,
    expiresAt: now + 1000,
    epoch,
    blockRevision: 0,
  };
}
test("quota freshness expires at its deadline and after thirty seconds without accepting future observations", () => {
  const quota = report();
  assert.equal(isQuotaFresh(quota, now), true);
  assert.equal(quotaLabel(quota, now), "25% remaining");
  assert.equal(isQuotaFresh(quota, quota.expiresAt), false);
  assert.equal(quotaLabel(quota, quota.expiresAt), "Quota stale");
  assert.equal(isQuotaFresh({ ...quota, observedAt: now - 30_000 }, now), true);
  assert.equal(
    isQuotaFresh({ ...quota, observedAt: now - 30_001 }, now),
    false,
  );
  assert.equal(isQuotaFresh({ ...quota, observedAt: now + 1 }, now), false);
});
test("partial quota windows cannot supply a remaining percentage", () => {
  const quota = report();
  for (const remainingPercent of [null, NaN, Infinity, -1, 101]) {
    const partial = {
      ...quota,
      windows: [
        quota.windows[0],
        { id: "missing", remainingPercent, resetAt: null },
      ],
    };
    assert.equal(isQuotaFresh(partial, now), false);
    assert.equal(quotaLabel(partial, now), "Quota unknown");
  }
  assert.equal(isQuotaFresh({ ...quota, windows: [] }, now), false);
});
test("balancing needs complete reports for every ready enabled router account in the same epoch", () => {
  const router: CliRouter = {
    id: "router",
    cli: "codex",
    label: "Pool",
    enabled: true,
    orderedProfileIds: ["first", "second"],
    balanceRemainingQuota: true,
    revision: 1,
  };
  const profiles: CliProfile[] = ["first", "second"].map((id) => ({
    id,
    cli: "codex",
    label: id,
    enabled: true,
    revision: 1,
    authState: "ready",
    storageMode: "cli_managed",
  }));
  const reports = [report(), report("second")];
  assert.equal(hasComparableQuota(router, profiles, reports, now), true);
  assert.equal(hasComparableQuota(router, profiles, [reports[0]], now), false);
  assert.equal(
    hasComparableQuota(
      router,
      profiles,
      [reports[0], report("second", 4)],
      now,
    ),
    false,
  );
  assert.equal(
    hasComparableQuota(router, profiles, reports, now + 1000),
    false,
  );
  assert.equal(
    hasComparableQuota(
      router,
      profiles.map((profile) => ({ ...profile, enabled: false })),
      reports,
      now,
    ),
    false,
  );
  assert.equal(
    hasComparableQuota(
      router,
      [profiles[0], { ...profiles[1], authState: "disconnected" }],
      [reports[0]],
      now,
    ),
    true,
  );
});

test("closing one CLI view preserves shared work until its final reference closes", () => {
  const views = [
    { id: "one", runId: "shared" },
    { id: "two", runId: "shared" },
    { id: "three", runId: "other" },
  ];
  assert.deepEqual(finalRunIds(views, new Set(["one"])), []);
  assert.deepEqual(finalRunIds(views, new Set(["one", "two"])), ["shared"]);
  assert.deepEqual(finalRunIds(views, new Set(["two", "three"])), ["other"]);
  assert.deepEqual(finalRunIds(views), ["shared", "other"]);
});
