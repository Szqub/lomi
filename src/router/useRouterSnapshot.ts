import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage } from "../api";
import { acceptSnapshot } from "./model";
import type { CliRouterSnapshot, RouterAction } from "./types";
export function useRouterSnapshot(poll = false) {
  const [snapshot, setSnapshot] = useState<CliRouterSnapshot>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const clearError = useCallback(() => setError(""), []);
  const live = useRef(false),
    lock = useRef(false);
  const latest = useRef<CliRouterSnapshot | undefined>(undefined);
  const readSnapshot = useCallback(() => latest.current, []);
  const accept = useCallback((next: CliRouterSnapshot) => {
    if (live.current) {
      latest.current = acceptSnapshot(latest.current, next);
      setSnapshot(latest.current);
    }
  }, []);
  const refresh = useCallback(async () => {
    try {
      accept(await api<CliRouterSnapshot>("cli_router_snapshot"));
    } catch (cause) {
      if (live.current) setError(errorMessage(cause));
    }
  }, [accept]);
  useEffect(() => {
    live.current = true;
    let active = true;
    let stop: (() => void) | undefined;
    void listen<{ revision: number }>("cli-router-changed", () => {
      if (active) void refresh();
    })
      .then((unlisten) => {
        if (!active) {
          unlisten();
          return;
        }
        stop = unlisten;
        void refresh();
      })
      .catch((cause) => {
        if (active) setError(errorMessage(cause));
      });
    return () => {
      active = false;
      live.current = false;
      stop?.();
    };
  }, [refresh]);
  useEffect(() => {
    if (!poll) return;
    const timer = window.setInterval(() => {
      void refresh();
    }, 1000);
    return () => window.clearInterval(timer);
  }, [poll, refresh]);
  const command = useCallback(
    async (name: string, args?: Record<string, unknown>) => {
      if (lock.current) return undefined;
      lock.current = true;
      setBusy(true);
      setError("");
      try {
        const next = await api<CliRouterSnapshot>(name, args);
        accept(next);
        return next;
      } catch (cause) {
        await refresh();
        if (live.current) setError(errorMessage(cause));
        return undefined;
      } finally {
        lock.current = false;
        if (live.current) setBusy(false);
      }
    },
    [accept, refresh],
  );
  const mutate = (action: RouterAction) =>
    command("cli_router_mutate", {
      request: {
        requestId: crypto.randomUUID(),
        expectedRevision: snapshot?.revision,
        action,
      },
    });
  return { snapshot, error, busy, command, mutate, clearError, readSnapshot };
}
