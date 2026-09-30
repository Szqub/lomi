import { ChildProcess } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { performance } from "node:perf_hooks";

// The caller marks the returned ChildProcess immediately after spawning it with
// detached:true. Node does not retain that spawn option on ChildProcess itself.
export async function terminateOwnedProcessGroup(
  child,
  { graceMs = 5000, forceMs = 5000 } = {},
) {
  if (
    !(child instanceof ChildProcess) ||
    child.detached !== true ||
    !Number.isSafeInteger(child.pid) ||
    child.pid <= 0 ||
    child.pid === process.pid
  ) {
    throw new Error(
      "Cleanup requires a known detached child with a positive PID.",
    );
  }
  for (const duration of [graceMs, forceMs]) {
    if (!Number.isFinite(duration) || duration < 0 || duration > 60_000) {
      throw new Error(
        "Process-group cleanup deadlines must be within 0–60000ms.",
      );
    }
  }
  const group = -child.pid;
  const signal = (name) => {
    try {
      process.kill(group, name);
      return true;
    } catch (error) {
      if (error.code === "ESRCH") return false;
      // macOS can report EPERM while a just-exited child awaits reaping. It
      // still means the group exists; never mistake it for successful cleanup.
      if (name === 0 && error.code === "EPERM") return true;
      throw error;
    }
  };
  const gone = async (duration) => {
    const deadline = performance.now() + duration;
    while (signal(0)) {
      const remaining = deadline - performance.now();
      if (remaining <= 0) return false;
      await delay(Math.min(25, remaining));
    }
    return true;
  };
  if (!signal("SIGTERM") || (await gone(graceMs))) return;
  if (!signal("SIGKILL") || (await gone(forceMs))) return;
  throw new Error(
    "Owned process group is still alive after forced cleanup; retain its app data.",
  );
}
