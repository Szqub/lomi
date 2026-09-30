import { useEffect, useRef, useState } from "react";
import { api, errorMessage, native } from "./api";
import type { GitRepositoryScan } from "./api";
import { containsPath, gitFilePath } from "./explorer-model";
import type { Project } from "./model";

export interface WorkspaceRepositorySummary {
  count: number;
  loading: boolean;
  limited: boolean;
  error?: string;
}

type ActiveScan = GitRepositoryScan & { loading: boolean };

export default function useWorkspaceRepositories(
  projects: readonly Project[],
  activeRoot: string,
  activeScan: ActiveScan,
): Record<string, WorkspaceRepositorySummary> {
  const rootsKey = JSON.stringify(
    [
      ...new Set(projects.map((project) => project.path).filter(Boolean)),
    ].sort(),
  );
  const cache = useRef(new Map<string, GitRepositoryScan>());
  const loading = useRef(new Set<string>());
  const inFlight = useRef(0);
  const pump = useRef<() => void>(() => {});
  const [, render] = useState(0);

  useEffect(() => {
    if (activeRoot && !activeScan.loading) {
      cache.current.set(activeRoot, {
        repositories: activeScan.repositories,
        errors: activeScan.errors,
        limited: activeScan.limited,
      });
    }
  }, [
    activeRoot,
    activeScan.repositories,
    activeScan.errors,
    activeScan.limited,
    activeScan.loading,
  ]);

  useEffect(() => {
    const roots: string[] = JSON.parse(rootsKey);
    const inactiveRoots = roots.filter((root) => root !== activeRoot);
    const membership = new Set(roots);
    for (const root of cache.current.keys()) {
      if (!membership.has(root)) cache.current.delete(root);
    }
    loading.current.clear();
    if (!native) return;

    let current = true;
    const pending = new Set<string>();
    const busy = new Set<string>();
    const update = async (root: string) => {
      inFlight.current += 1;
      busy.add(root);
      loading.current.add(root);
      render((value) => value + 1);
      const previous = cache.current.get(root);
      try {
        const result = await api<GitRepositoryScan>("git_repositories", {
          root,
        });
        if (!current) return;
        // Incomplete discovery must not discard previously discovered repositories.
        const retained = (previous?.repositories ?? []).filter(
          (repo) =>
            !result.repositories.some((next) => next.root === repo.root) &&
            (result.limited ||
              result.errors.some((error) =>
                containsPath(gitFilePath(error.root), gitFilePath(repo.root)),
              )),
        );
        cache.current.set(root, {
          ...result,
          repositories: [...result.repositories, ...retained],
        });
      } catch (error) {
        if (!current) return;
        cache.current.set(root, {
          repositories: previous?.repositories ?? [],
          limited: previous?.limited ?? false,
          errors: [{ root, message: errorMessage(error) }],
        });
      } finally {
        inFlight.current -= 1;
        busy.delete(root);
        if (current) {
          loading.current.delete(root);
          render((value) => value + 1);
        }
        pump.current();
      }
    };
    const drain = () => {
      if (!current || document.visibilityState === "hidden") return;
      for (const root of pending) {
        if (inFlight.current >= 2) break;
        if (busy.has(root)) continue;
        pending.delete(root);
        void update(root);
      }
    };
    const refresh = () => {
      if (document.visibilityState === "hidden") return;
      for (const root of inactiveRoots) {
        if (!busy.has(root)) pending.add(root);
      }
      drain();
    };
    pump.current = drain;
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      current = false;
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
      pump.current = () => {};
    };
  }, [rootsKey, activeRoot]);

  const summaries: Record<string, WorkspaceRepositorySummary> = {};
  for (const root of JSON.parse(rootsKey) as string[]) {
    const scan =
      root === activeRoot && !activeScan.loading
        ? activeScan
        : cache.current.get(root);
    summaries[root] = {
      count: new Set(scan?.repositories.map((repo) => repo.root)).size,
      loading:
        (root === activeRoot && activeScan.loading) ||
        loading.current.has(root) ||
        (!scan && native),
      limited: scan?.limited ?? false,
      error: scan?.errors.length
        ? scan.errors.map((error) => error.message).join("\n")
        : undefined,
    };
  }
  return summaries;
}
