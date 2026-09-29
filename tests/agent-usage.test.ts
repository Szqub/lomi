import assert from "node:assert/strict";
import test from "node:test";
import { cliNames } from "../src/cli-agents.ts";
import {
  agentUsageSignature,
  agentUsageTargetBatches,
  agentUsageTargets,
  isCliAgent,
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
