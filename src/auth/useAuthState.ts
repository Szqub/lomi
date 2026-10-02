import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, native } from "../api";
import { newestAuthState, unavailableState } from "./model";
import type { AuthState } from "./model";

export function useAuthState() {
  const [state, setState] = useState<AuthState | null>(
    native ? null : unavailableState,
  );
  const current = useRef(state);
  useEffect(() => {
    if (!native) return;
    let active = true;
    const initial = current.current;
    let stop: (() => void) | undefined;
    const accept = (incoming: AuthState) => {
      if (!active) return;
      const next = newestAuthState(current.current, incoming);
      current.current = next;
      setState(next);
    };
    void (async () => {
      try {
        const unlisten = await listen<AuthState>(
          "auth-state-changed",
          ({ payload }) => accept(payload),
        );
        if (!active) {
          unlisten();
          return;
        }
        stop = unlisten;
        accept(await api<AuthState>("auth_get_state"));
      } catch {
        if (active && current.current === initial) {
          const unavailable = {
            ...unavailableState,
            revision: current.current?.revision ?? 0,
          };
          current.current = unavailable;
          setState(unavailable);
        }
      }
    })();
    return () => {
      active = false;
      stop?.();
    };
  }, []);
  return { state, current };
}
