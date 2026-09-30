import { api } from "./api";
import type { CliAgent } from "./cli-agents";
import type { ShellProfile } from "./model";

export interface InstalledCli {
  cli: CliAgent;
  name: string;
  command: string;
}

const freshness = 5 * 60 * 1000;
const capacity = 16;
const cache = new Map<string, { clients: InstalledCli[]; updated: number }>();
const pending = new Map<string, Promise<InstalledCli[]>>();
const forced = new Map<string, Promise<InstalledCli[]>>();

function key(profile: ShellProfile, cwd: string) {
  return JSON.stringify([profile, cwd]);
}

export function cachedInstalledAgentClis(profile: ShellProfile, cwd: string) {
  const identity = key(profile, cwd);
  const entry = cache.get(identity);
  if (entry) {
    cache.delete(identity);
    cache.set(identity, entry);
  }
  return entry?.clients;
}

export function loadInstalledAgentClis(
  profile: ShellProfile,
  cwd: string,
  force = false,
): Promise<InstalledCli[]> {
  const identity = key(profile, cwd);
  const queued = forced.get(identity);
  if (queued) return queued;
  const running = pending.get(identity);
  if (running) {
    if (!force) return running;
    // An explicit refresh must observe changes made after the pending scan began.
    const refresh = running
      .catch(() => undefined)
      .then(() => {
        forced.delete(identity);
        return loadInstalledAgentClis(profile, cwd, true);
      });
    forced.set(identity, refresh);
    return refresh;
  }
  const entry = cache.get(identity);
  if (!force && entry && Date.now() - entry.updated < freshness) {
    return Promise.resolve(cachedInstalledAgentClis(profile, cwd)!);
  }
  const scan = api<InstalledCli[]>("installed_agent_clis", {
    profileId: profile.id,
    cwd,
  }).then(
    (clients) => {
      pending.delete(identity);
      cache.delete(identity);
      cache.set(identity, { clients, updated: Date.now() });
      while (cache.size > capacity) cache.delete(cache.keys().next().value!);
      return clients;
    },
    (cause: unknown) => {
      pending.delete(identity);
      throw cause;
    },
  );
  pending.set(identity, scan);
  return scan;
}
