import { listen } from "@tauri-apps/api/event";
import { closeGatewayTerminals } from "../terminal-runtime";
import { api, errorMessage } from "../api";
import { cliAgentTabs, type CliAgentTab, type Session } from "../model";
import { acceptSnapshot, finalRunIds, isRunning } from "./model";
import type { CliRouterSnapshot } from "./types";

interface RunViewState {
  snapshot?: CliRouterSnapshot;
  loaded: boolean;
  error: string;
  busy: boolean;
  draft: string;
}
class RunView {
  state: RunViewState = { loaded: false, error: "", busy: false, draft: "" };
  closing?: symbol;
  private commandBusy = false;
  private listeners = new Set<() => void>();
  constructor(readonly runId: string) {}
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  getSnapshot = () => this.state;
  update(change: Partial<RunViewState>) {
    this.state = {
      ...this.state,
      ...change,
      busy: this.commandBusy || this.closing !== undefined,
    };
    for (const listener of this.listeners) listener();
  }
  setDraft = (draft: string) => this.update({ draft });
  async command(name: string, args: Record<string, unknown>) {
    if (this.state.busy) return;
    this.commandBusy = true;
    this.update({ error: "" });
    try {
      const next = await api<CliRouterSnapshot>(name, args);
      accept(next);
      return next;
    } catch (cause) {
      await refresh();
      this.update({ error: errorMessage(cause) });
    } finally {
      this.commandBusy = false;
      this.update({});
    }
  }
}
let retained: CliAgentTab[] = [];
let snapshot: CliRouterSnapshot | undefined;
const views = new Map<string, RunView>();
let generation = 0;
let stop: (() => void) | undefined;
let poll: ReturnType<typeof setInterval> | undefined;
let subscribed = false;
function accept(next: CliRouterSnapshot) {
  snapshot = acceptSnapshot(snapshot, next);
  for (const view of views.values()) view.update({ snapshot, loaded: true });
  updatePoll();
}
async function refresh() {
  try {
    accept(await api<CliRouterSnapshot>("cli_router_snapshot"));
  } catch (cause) {
    for (const view of views.values())
      view.update({ error: errorMessage(cause) });
  }
}
function updatePoll() {
  const running = retained.some((tab) =>
    snapshot?.runs.some((run) => run.id === tab.runId && isRunning(run.state)),
  );
  if (running && !poll)
    poll = setInterval(() => {
      void refresh();
    }, 1000);
  if (!running && poll) {
    clearInterval(poll);
    poll = undefined;
  }
}
function connect() {
  if (subscribed || !retained.length) return;
  subscribed = true;
  const epoch = ++generation;
  void listen<{ revision: number }>("cli-router-changed", () => {
    if (epoch === generation) void refresh();
  })
    .then((unlisten) => {
      if (epoch !== generation) {
        unlisten();
        return;
      }
      stop = unlisten;
      void refresh();
    })
    .catch((cause) => {
      if (epoch === generation) {
        subscribed = false;
        for (const view of views.values())
          view.update({ error: errorMessage(cause) });
      }
    });
}
export function getRunView(runId: string) {
  let view = views.get(runId);
  if (!view) {
    view = new RunView(runId);
    if (snapshot) view.update({ snapshot, loaded: true });
    views.set(runId, view);
  }
  connect();
  return view;
}
export function retainCliAgents(session?: Session) {
  retained = cliAgentTabs(session);
  const ids = new Set(retained.map((tab) => tab.runId));
  for (const id of ids) getRunView(id);
  const removedClosing: string[] = [];
  for (const [id, view] of views) {
    if (ids.has(id)) continue;
    if (view.closing) removedClosing.push(id);
    views.delete(id);
  }
  if (removedClosing.length)
    void api("cli_run_close_release", { runIds: removedClosing }).catch(
      () => {},
    );
  if (!ids.size) {
    ++generation;
    subscribed = false;
    stop?.();
    stop = undefined;
    snapshot = undefined;
  }
  updatePoll();
}
export function closingCliRunIds(panelIds?: ReadonlySet<string>) {
  return finalRunIds(retained, panelIds);
}
export function hasActiveCliRuns(panelIds?: ReadonlySet<string>) {
  return closingCliRunIds(panelIds).some((id) =>
    snapshot?.runs.some((run) => run.id === id && isRunning(run.state)),
  );
}
export interface CliCloseLease {
  release: () => Promise<void>;
}
export async function closeCliAgentViews(
  panelIds: ReadonlySet<string>,
): Promise<CliCloseLease> {
  // A missing history record has no process to drain; native snapshot verifies that
  // rather than treating an unmounted or not-yet-loaded view as idle.
  const ids = closingCliRunIds(panelIds);
  if (!ids.length) return { release: async () => {} };
  const entries = ids.map(getRunView);
  if (entries.some((entry) => entry.closing))
    throw new Error("These CLI Agent views are already closing.");
  const owner = Symbol("CLI view close");
  for (const entry of entries) {
    entry.closing = owner;
    entry.update({});
  }
  const lease: CliCloseLease = {
    release: async () => {
      const pending = entries.filter(
        (entry) => views.get(entry.runId) === entry && entry.closing === owner,
      );
      if (!pending.length) return;
      await api("cli_run_close_release", {
        runIds: pending.map((entry) => entry.runId),
      });
      for (const entry of pending) {
        if (entry.closing !== owner) continue;
        entry.closing = undefined;
        entry.update({});
      }
    },
  };
  try {
    const latest = await api<CliRouterSnapshot>("cli_router_snapshot");
    accept(latest);
    for (const runId of ids) {
      if (!latest.runs.some((run) => run.id === runId)) continue;
      accept(await api<CliRouterSnapshot>("cli_run_drain", { runId }));
      closeGatewayTerminals([runId]);
    }
    return lease;
  } catch (error) {
    await lease.release();
    throw error;
  }
}
