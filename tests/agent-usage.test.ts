import assert from "node:assert/strict";
import test from "node:test";
import { cliNames } from "../src/cli-agents.ts";
import {
  agentUsageSignature,
  agentUsageTargetBatches,
  agentUsageTargets,
  isCliAgent,
  groupAgentUsage,
  worstRemainingWindow,
} from "../src/agent-usage.ts";
import type { AgentUsageEntry } from "../src/agent-usage.ts";
import type { TerminalContext } from "../src/terminal-runtime";

function context(cli: string, pid: number): TerminalContext {
  return {
    cwd: "/project",
    foregroundProgram: cli,
    titleCli: { cli, pid } as TerminalContext["titleCli"],
  };
}

test("usage targets include every supported CLI in hidden and background terminals", () => {
  const contexts = Object.fromEntries(
    Object.keys(cliNames).map((cli, index) => [
      `terminal-${index}`,
      context(cli, index + 1),
    ]),
  ) as Record<string, TerminalContext>;
  contexts.unrelated = { ...context("codex", 50), titleCli: null };
  contexts.invalid = context("unknown-cli", 51);
  contexts.invalidPid = context("codex", 0);

  assert.equal(Object.keys(cliNames).length, 31);
  assert.equal(agentUsageTargets(contexts).length, 31);
  assert.deepEqual(
    new Set(agentUsageTargets(contexts).map((target) => target.process.cli)),
    new Set(Object.keys(cliNames)),
  );
  assert.equal(isCliAgent("sweagent"), true);
  assert.equal(isCliAgent("unknown-cli"), false);
});

test("usage signatures change when a process changes and ignore object order", () => {
  const first = agentUsageTargets({
    a: context("codex", 12),
    b: context("agy", 7),
  });
  const reordered = agentUsageTargets({
    b: context("agy", 7),
    a: context("codex", 12),
  });
  const switched = agentUsageTargets({
    a: context("codex", 13),
    b: context("agy", 7),
  });

  assert.equal(agentUsageSignature(first), agentUsageSignature(reordered));
  assert.notEqual(agentUsageSignature(first), agentUsageSignature(switched));
});

test("usage requests are split into bounded batches without dropping targets", () => {
  const contexts = Object.fromEntries(
    Array.from({ length: 260 }, (_, index) => [
      `terminal-${index}`,
      context("codex", index + 1),
    ]),
  ) as Record<string, TerminalContext>;
  const batches = agentUsageTargetBatches(agentUsageTargets(contexts));

  assert.deepEqual(
    batches.map((batch) => batch.length),
    [128, 128, 4],
  );
  assert.equal(batches.flat().length, 260);
  assert.equal(new Set(batches.flat().map((target) => target.id)).size, 260);
});

test("the summary chooses the lowest valid remaining quota and ignores unknown values", () => {
  const entries: AgentUsageEntry[] = [
    {
      id: "codex-pane",
      process: { cli: "codex", pid: 12 },
      status: "ready",
      windows: [
        {
          label: "5-hour",
          remainingPercent: 64,
          used: 180,
          limit: 500,
          unit: "requests",
          resetsAt: null,
        },
        {
          label: "Weekly",
          remainingPercent: null,
          used: 140,
          limit: null,
          unit: "requests",
          resetsAt: null,
        },
      ],
      updatedAt: 1000,
      retryAt: null,
      message: null,
      source: "Account",
    },
    {
      id: "claude-pane",
      process: { cli: "claude", pid: 13 },
      status: "error",
      windows: [
        {
          label: "Monthly",
          remainingPercent: 18.5,
          used: 815,
          limit: 1000,
          unit: "tokens",
          resetsAt: 2_000,
        },
      ],
      updatedAt: 900,
      retryAt: null,
      message: "Internal detail stays native-only",
      source: null,
    },
  ];

  assert.equal(worstRemainingWindow(entries)?.entry.id, "claude-pane");
  assert.equal(worstRemainingWindow(entries)?.window.label, "Monthly");
  assert.equal(worstRemainingWindow([]), null);
});

function accountEntry(
  id: string,
  accountKey?: string | null,
  options: Partial<AgentUsageEntry> = {},
): AgentUsageEntry {
  return {
    id,
    process: { cli: "codex", pid: Number(id) },
    accountKey,
    status: "ready",
    windows: [
      {
        label: "Weekly",
        remainingPercent: 70,
        used: null,
        limit: null,
        unit: null,
        resetsAt: null,
      },
    ],
    updatedAt: 1000,
    retryAt: null,
    message: null,
    source: null,
    ...options,
  };
}

function targetsFor(entries: AgentUsageEntry[]) {
  return entries.map(({ id, process }) => ({ id, process }));
}

test("four CLI sessions share one account row while requests retain all sessions", () => {
  const entries = [1, 2, 3, 4].map((id) =>
    accountEntry(String(id), "account-a"),
  );
  const targets = targetsFor(entries);
  const groups = groupAgentUsage(targets, entries);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].entry?.id, "1");
  assert.equal(agentUsageTargetBatches(targets).flat().length, 4);
});

test("different accounts keep separate rows even with identical quota values", () => {
  const entries = [
    accountEntry("1", "account-a"),
    accountEntry("2", "account-b"),
  ];
  assert.equal(groupAgentUsage(targetsFor(entries), entries).length, 2);
});

test("account grouping is scoped to the CLI provider", () => {
  const entries = [
    accountEntry("1", "shared"),
    accountEntry("2", "shared", { process: { cli: "claude", pid: 2 } }),
  ];
  assert.equal(groupAgentUsage(targetsFor(entries), entries).length, 2);
});

test("unknown, missing and empty account identities each retain their session row", () => {
  const entries = [
    accountEntry("1", null),
    accountEntry("2"),
    accountEntry("3", ""),
    accountEntry("4", null),
  ];
  const targets = [
    ...targetsFor(entries),
    { id: "5", process: { cli: "codex" as const, pid: 5 } },
  ];
  const groups = groupAgentUsage(targets, entries);
  assert.equal(groups.length, 5);
  assert.equal(groups[4].entry, null);
});

test("account rows prefer usable snapshots, then latest updates with deterministic ties", () => {
  const saved = accountEntry("1", "shared", {
    status: "error",
    updatedAt: 100,
  });
  const empty = accountEntry("2", "shared", { windows: [], updatedAt: 400 });
  const failed = accountEntry("3", "shared", {
    windows: [],
    status: "error",
    updatedAt: 500,
  });
  const ready = accountEntry("4", "shared", { updatedAt: 200 });
  const latest = accountEntry("5", "shared", { updatedAt: 300 });
  const tie = accountEntry("6", "shared", { updatedAt: 300 });
  const choose = (entries: AgentUsageEntry[]) =>
    groupAgentUsage(targetsFor(entries), entries)[0].entry;
  assert.equal(choose([empty, failed, saved]), saved);
  assert.equal(choose([saved, ready]), ready);
  assert.equal(choose([ready, latest]), latest);
  assert.equal(choose([tie, latest]), latest);
  assert.equal(choose([latest, tie]), latest);
  const invalid = accountEntry("7", "shared", {
    windows: [{ ...saved.windows[0], remainingPercent: NaN, used: Infinity }],
    updatedAt: 600,
  });
  assert.equal(choose([invalid, saved]), saved);
});

test("newer saved usage wins over older ready usage and ready wins equal-time ties", () => {
  const ready = accountEntry("1", "shared", {
    windows: [
      {
        label: "Weekly",
        remainingPercent: 50,
        used: null,
        limit: null,
        unit: null,
        resetsAt: null,
      },
    ],
    updatedAt: 1000,
  });
  const saved = accountEntry("2", "shared", {
    status: "rate-limited",
    windows: [{ ...ready.windows[0], remainingPercent: 40 }],
    updatedAt: 2000,
  });
  const groups = groupAgentUsage(targetsFor([ready, saved]), [ready, saved]);
  assert.equal(groups[0].entry?.windows[0].remainingPercent, 40);
  assert.equal(groups[0].entry?.status, "rate-limited");
  const equalTimeReady = { ...ready, updatedAt: saved.updatedAt };
  assert.equal(
    groupAgentUsage(targetsFor([saved, equalTimeReady]), [
      saved,
      equalTimeReady,
    ])[0].entry,
    equalTimeReady,
  );
  assert.equal(
    groupAgentUsage(targetsFor([equalTimeReady, saved]), [
      equalTimeReady,
      saved,
    ])[0].entry,
    equalTimeReady,
  );
});

test("removed and replaced processes cannot supply identities or snapshots to current rows", () => {
  const entries = [
    accountEntry("1", "shared", { updatedAt: 2000 }),
    accountEntry("2", "shared"),
  ];
  assert.equal(
    groupAgentUsage(targetsFor(entries).slice(1), entries)[0].entry?.id,
    "2",
  );
  const targets = [
    { id: "1", process: { cli: "codex" as const, pid: 99 } },
    targetsFor(entries)[1],
  ];
  const groups = groupAgentUsage(targets, entries);
  assert.equal(groups.length, 2);
  assert.equal(groups[0].entry, null);
  assert.equal(groups[1].entry?.id, "2");
  assert.equal(
    groupAgentUsage(
      [{ id: "2", process: { cli: "claude", pid: 2 } }],
      entries,
    )[0].entry,
    null,
  );
});
