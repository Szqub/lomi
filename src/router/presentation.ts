import type { RunState } from "./types";

export const runStateLabels: Record<RunState, string> = {
  idle: "Ready",
  starting: "Starting…",
  running: "Working",
  switching: "Switching account…",
  waiting_for_capacity: "Waiting for quota",
  paused: "Paused",
  completed: "Completed",
  stopped: "Stopped",
  failed: "Failed",
  recovery_required: "Review needed",
};
