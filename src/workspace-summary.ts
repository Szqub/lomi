import type { CliAgent } from "./cli-agents.ts";
import { panes } from "./model.ts";
import type { Workspace } from "./model.ts";

export interface WorkspaceAgentGroup {
  cli: CliAgent;
  count: number;
}
export interface WorkspaceAgentSummary {
  total: number;
  groups: WorkspaceAgentGroup[];
  byTab: Record<string, WorkspaceAgentGroup[]>;
}

export function summarizeWorkspaceAgents(
  workspace: Workspace,
  agents: Readonly<Record<string, CliAgent>>,
): WorkspaceAgentSummary {
  const seen = new Set<string>();
  const counts = new Map<CliAgent, number>();
  const byTab: Record<string, WorkspaceAgentGroup[]> = {};
  let total = 0;
  for (const tab of workspace.tabs) {
    const tabCounts = new Map<CliAgent, number>();
    if (tab.type === "terminal") {
      for (const pane of panes(tab.layout)) {
        if (seen.has(pane.id)) continue;
        seen.add(pane.id);
        const cli = Object.hasOwn(agents, pane.id)
          ? agents[pane.id]
          : undefined;
        if (!cli) continue;
        total++;
        counts.set(cli, (counts.get(cli) ?? 0) + 1);
        tabCounts.set(cli, (tabCounts.get(cli) ?? 0) + 1);
      }
    }
    Object.defineProperty(byTab, tab.id, {
      value: [...tabCounts].map(([cli, count]) => ({ cli, count })),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return {
    total,
    groups: [...counts].map(([cli, count]) => ({ cli, count })),
    byTab,
  };
}
