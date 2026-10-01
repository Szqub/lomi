import { panes, tabTitle } from "./model.ts";
import type { Layout, Pane, Session, ShellProfile } from "./model.ts";

export interface RemoteWorkspaceStatus {
  id: string;
  shared: boolean;
  online: boolean;
  message: string | null;
}
export interface RemoteState {
  qualified: boolean;
  enabled: boolean;
  online: boolean;
  message: string | null;
  domainEpoch: string | null;
  workspaces: RemoteWorkspaceStatus[];
  grants: {
    id: string;
    fingerprint: string;
    sessionIds: string[];
    permissions: "observe" | "control";
    expiresAt: number;
    revoked: boolean;
  }[];
}
export type RemoteLayout =
  | { type: "terminal"; paneId: string }
  | {
      type: "split";
      axis: "horizontal" | "vertical";
      ratio: number;
      first: RemoteLayout;
      second: RemoteLayout;
    };
export interface RemoteTab {
  id: string;
  title: string;
  layout: RemoteLayout;
}
export interface WorkspaceProjection {
  id: string;
  name: string;
  tabs?: RemoteTab[];
  terminals: { paneId: string; sessionId: string | null; title: string }[];
}
function terminalLayout(layout: Layout): RemoteLayout | undefined {
  if (layout.type === "terminal")
    return { type: "terminal", paneId: layout.id };
  if (layout.type !== "split") return undefined;
  const first = terminalLayout(layout.first);
  const second = terminalLayout(layout.second);
  if (!first) return second;
  if (!second) return first;
  return {
    type: "split",
    axis: layout.axis,
    ratio:
      Number.isFinite(layout.ratio) && layout.ratio > 0 && layout.ratio < 1
        ? layout.ratio
        : 0.5,
    first,
    second,
  };
}
export function workspaceTerminals(session: Session | undefined) {
  return (
    session?.projects.flatMap((project) =>
      project.workspaces.map((workspace) => ({
        id: workspace.id,
        name: workspace.name,
        tabs: workspace.tabs.flatMap((tab) => {
          if (tab.type !== "terminal") return [];
          const layout = terminalLayout(tab.layout);
          return layout ? [{ id: tab.id, title: tabTitle(tab), layout }] : [];
        }),
        terminals: workspace.tabs.flatMap((tab) =>
          tab.type === "terminal"
            ? panes(tab.layout).map((pane) => ({
                pane,
                profileId: pane.profileId ?? tab.profileId,
                title: tabTitle(tab),
              }))
            : [],
        ),
      })),
    ) ?? []
  );
}

/** One publisher owns the native epoch and never publishes a captured domain after awaits. */
export class WorkspacePublisher {
  private epoch?: string;
  private revision = 0;
  private dirty = false;
  private stopped = false;
  private pending?: Promise<void>;
  private last = "";
  readonly desired = new Set<string>();
  private readonly ports: {
    session: () => Session | undefined;
    profiles: () => ShellProfile[];
    sessionId: (paneId: string) => string | null;
    start: (pane: Pane, profile: ShellProfile) => Promise<void>;
    begin: () => Promise<{ epoch: string }>;
    sync: (
      epoch: string,
      revision: number,
      workspaces: WorkspaceProjection[],
    ) => Promise<void>;
  };
  constructor(ports: WorkspacePublisher["ports"]) {
    this.ports = ports;
  }
  stop() {
    this.stopped = true;
  }
  notify() {
    this.dirty = true;
    if (!this.pending) {
      this.pending = this.drain().finally(() => {
        this.pending = undefined;
      });
    }
    return this.pending;
  }
  private async drain() {
    if (!this.epoch) this.epoch = (await this.ports.begin()).epoch;
    while (this.dirty && !this.stopped) {
      this.dirty = false;
      const projection = workspaceTerminals(this.ports.session()).map(
        (workspace) => ({
          id: workspace.id,
          name: workspace.name,
          tabs: workspace.tabs,
          terminals: workspace.terminals.map(({ pane, title }) => ({
            paneId: pane.id,
            sessionId: this.ports.sessionId(pane.id),
            title,
          })),
        }),
      );
      const key = JSON.stringify(projection);
      if (key !== this.last) {
        await this.ports.sync(this.epoch, ++this.revision, projection);
        this.last = key;
      }
      if (this.stopped) return;
      // Read current domain before each start: deletion, moves, and unshare can happen during a start.
      const candidates = workspaceTerminals(this.ports.session())
        .filter((w) => this.desired.has(w.id))
        .flatMap((w) => w.terminals.map((t) => ({ ...t, workspaceId: w.id })));
      for (const candidate of candidates) {
        if (this.stopped) return;
        const latest = workspaceTerminals(this.ports.session()).find(
          (w) => w.id === candidate.workspaceId,
        );
        const terminal = latest?.terminals.find(
          (t) => t.pane.id === candidate.pane.id,
        );
        if (!this.desired.has(candidate.workspaceId) || !terminal) continue;
        const profile = this.ports
          .profiles()
          .find((p) => p.id === terminal.profileId);
        if (!profile)
          throw new Error("The terminal shell profile is unavailable.");
        const before = this.ports.sessionId(terminal.pane.id);
        await this.ports.start(terminal.pane, profile);
        if (this.ports.sessionId(terminal.pane.id) !== before)
          this.dirty = true;
      }
    }
  }
}
