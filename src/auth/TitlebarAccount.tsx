import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "../api";
import { Github } from "../icons";
import { authStatusLabel, newestAuthState, unavailableState } from "./model";
import type { AuthState } from "./model";
import "./titlebar-account.css";

export default function TitlebarAccount({
  onError,
}: {
  onError: (message: string) => void;
}) {
  const [state, setState] = useState<AuthState | null>(
    native ? null : unavailableState,
  );
  const current = useRef(state);
  const mounted = useRef(false);
  const pending = useRef(false);
  const [opening, setOpening] = useState(false);
  const [failedAvatar, setFailedAvatar] = useState<string | null>(null);
  const accept = useCallback((incoming: AuthState) => {
    const next = newestAuthState(current.current, incoming);
    if (next !== current.current) {
      current.current = next;
      setState(next);
    }
    return next;
  }, []);

  useEffect(() => {
    mounted.current = true;
    let active = true;
    let stop: (() => void) | undefined;

    if (!native) {
      return () => {
        active = false;
        mounted.current = false;
      };
    }

    void (async () => {
      try {
        const unlisten = await listen<AuthState>(
          "auth-state-changed",
          ({ payload }) => {
            if (active) accept(payload);
          },
        );
        if (!active) {
          unlisten();
          return;
        }
        stop = unlisten;
        const snapshot = await api<AuthState>("auth_get_state");
        if (active) accept(snapshot);
      } catch {
        if (active) accept(unavailableState);
      }
    })();

    return () => {
      active = false;
      mounted.current = false;
      stop?.();
    };
  }, [accept]);

  const account = state?.user ?? null;
  const hasAccount = Boolean(account || state?.session);
  const login = account?.githubLogin?.trim() || null;
  const displayName = account?.displayName.trim() || login || "";
  const initials = displayName
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0])
    .join("")
    .toLocaleUpperCase();
  const status = state ? authStatusLabel(state.status) : "Checking account…";
  const buttonLabel = account
    ? `Open account settings for ${displayName || "your account"}`
    : hasAccount
      ? "Open account settings"
      : "Sign In";
  const title = opening
    ? hasAccount
      ? "Opening account settings…"
      : "Starting sign-in…"
    : !native
      ? "Sign-in is available in the Lomi desktop app"
      : hasAccount
        ? `${buttonLabel} · ${status}`
        : `Sign in to your Lomi account · ${status}`;

  const openAccount = () => {
    if (!native || pending.current) return;
    pending.current = true;
    setOpening(true);
    void (async () => {
      try {
        if (hasAccount) {
          await api("open_settings", { page: "account" });
          return;
        }

        let attempt =
          current.current?.status === "authorizing"
            ? current.current.attempt
            : null;
        if (!attempt) {
          const started = await api<AuthState>("auth_begin_login", {
            storage: "persistent",
          });
          if (!mounted.current) return;
          const latest = accept(started);
          if (latest && latest.revision > started.revision) return;
          if (started.status === "unavailable" || !started.attempt) {
            if (
              started.status === "unavailable" ||
              started.status === "error" ||
              started.status === "storage-locked"
            ) {
              onError(started.message || "Sign-in is not available right now.");
            }
            return;
          }
          if (
            latest?.status !== "authorizing" ||
            latest.attempt?.id !== started.attempt.id
          ) {
            return;
          }
          attempt = latest.attempt;
        }

        if (!attempt || !mounted.current) return;
        const latest = current.current;
        if (
          latest?.status !== "authorizing" ||
          latest.attempt?.id !== attempt.id
        ) {
          return;
        }

        const opened = await api<AuthState>("auth_open_verification", {
          attemptId: attempt.id,
        });
        if (!mounted.current) return;
        const accepted = accept(opened);
        if (
          opened.status === "unavailable" &&
          !opened.attempt &&
          accepted === opened
        ) {
          onError(opened.message || "The sign-in page could not be opened.");
        }
      } catch (error) {
        if (mounted.current) onError(errorMessage(error));
      } finally {
        pending.current = false;
        if (mounted.current) setOpening(false);
      }
    })();
  };

  return (
    <button
      type="button"
      className={`titlebar-account${hasAccount ? " titlebar-account-avatar" : ""}`}
      title={title}
      aria-label={buttonLabel}
      aria-busy={opening || undefined}
      disabled={!native || opening}
      onClick={openAccount}
    >
      {hasAccount ? (
        <span className="titlebar-account-image" aria-hidden="true">
          {login && failedAvatar !== login ? (
            <img
              src={`https://avatars.githubusercontent.com/${encodeURIComponent(login)}?s=64`}
              referrerPolicy="no-referrer"
              alt=""
              onError={() => setFailedAvatar(login)}
            />
          ) : initials ? (
            <span>{initials}</span>
          ) : (
            <Github size={13} />
          )}
        </span>
      ) : (
        "Sign In"
      )}
    </button>
  );
}
