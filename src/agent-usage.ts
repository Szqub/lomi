import { cliNames, type CliAgent } from "./cli-agents.ts";
import type { TerminalContext, TitleProcess } from "./terminal-runtime";

export type AgentUsageStatus =
  "ready" | "unauthenticated" | "unsupported" | "error" | "rate-limited";

export interface AgentUsageWindow {
  label: string;
  /** Remaining quota: 0 means exhausted, 100 means fully available. Native adapters normalize providers to this scale. */
  remainingPercent: number | null;
  used: number | null;
  limit: number | null;
  unit: string | null;
  resetsAt: number | null;
}

export interface AgentUsageEntry {
  id: string;
  process: TitleProcess;
  status: AgentUsageStatus;
  windows: AgentUsageWindow[];
  updatedAt: number | null;
  retryAt: number | null;
  message: string | null;
  source: string | null;
}

export interface AgentUsageTarget {
  id: string;
  process: TitleProcess;
}

export const maxUsageTargetsPerRequest = 128;

export function isCliAgent(value: unknown): value is CliAgent {
  return typeof value === "string" && Object.hasOwn(cliNames, value);
}

export function agentUsageTargets(
  contexts: Record<string, TerminalContext>,
): AgentUsageTarget[] {
  return Object.entries(contexts)
    .flatMap(([id, context]) => {
      const process = context?.titleCli;
      if (
        !process ||
        !isCliAgent(process.cli) ||
        !Number.isSafeInteger(process.pid) ||
        process.pid <= 0
      )
        return [];
      return [{ id, process }];
    })
    .sort((left, right) => left.id.localeCompare(right.id));
}

export function agentUsageSignature(targets: AgentUsageTarget[]): string {
  return JSON.stringify(
    targets.map(({ id, process }) => [id, process.cli, process.pid]),
  );
}

export function agentUsageTargetBatches(targets: AgentUsageTarget[]) {
  const batches: AgentUsageTarget[][] = [];
  for (
    let index = 0;
    index < targets.length;
    index += maxUsageTargetsPerRequest
  )
    batches.push(targets.slice(index, index + maxUsageTargetsPerRequest));
  return batches;
}

export function validRemainingPercent(value: number | null): value is number {
  return value !== null && Number.isFinite(value) && value >= 0 && value <= 100;
}

export function worstRemainingWindow(
  entries: AgentUsageEntry[],
): { entry: AgentUsageEntry; window: AgentUsageWindow } | null {
  let worst: { entry: AgentUsageEntry; window: AgentUsageWindow } | null = null;
  for (const entry of entries) {
    for (const window of entry.windows) {
      if (
        validRemainingPercent(window.remainingPercent) &&
        (!worst || window.remainingPercent < worst.window.remainingPercent!)
      )
        worst = { entry, window };
    }
  }
  return worst;
}
