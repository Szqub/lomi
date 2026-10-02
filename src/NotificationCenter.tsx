import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { RefObject } from "react";
import { createPortal } from "react-dom";
import { Bell, Check, CircleAlert, CircleCheck, Trash2, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "./api";
import {
  clearNotificationError,
  newestNotificationSnapshot,
  subscribeNotificationErrors,
  unreadNotificationCount,
} from "./notifications";
import type { NotificationSnapshot } from "./notifications";
import "./notification-center.css";

export function useNotifications() {
  const [snapshot, setSnapshot] = useState<NotificationSnapshot | null>(null);
  const current = useRef<NotificationSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const errorVersion = useRef(0);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const mounted = useRef(false);
  const [attempt, setAttempt] = useState(0);
  const accept = useCallback((incoming: NotificationSnapshot) => {
    const next = newestNotificationSnapshot(current.current, incoming);
    current.current = next;
    setSnapshot(next);
  }, []);
  useEffect(() => {
    mounted.current = true;
    let active = true;
    let stop: (() => void) | undefined;
    const stopErrors = subscribeNotificationErrors((message) => {
      errorVersion.current++;
      setError(message);
    });
    const startedErrorVersion = ++errorVersion.current;
    if (native)
      void (async () => {
        try {
          const unlisten = await listen<NotificationSnapshot>(
            "notifications-changed",
            ({ payload }) => {
              if (active) accept(payload);
            },
          );
          if (!active) {
            unlisten();
            return;
          }
          stop = unlisten;
          const incoming =
            await api<NotificationSnapshot>("load_notifications");
          if (active) {
            accept(incoming);
            if (errorVersion.current === startedErrorVersion && attempt > 0) {
              setError(null);
              clearNotificationError();
            }
          }
        } catch (failure) {
          if (active && errorVersion.current === startedErrorVersion)
            setError(`Could not load notifications: ${errorMessage(failure)}`);
        }
      })();
    return () => {
      active = false;
      mounted.current = false;
      stop?.();
      stopErrors();
    };
  }, [accept, attempt]);

  const mutate = async (command: string, args?: Record<string, unknown>) => {
    if (!native || pending.current) return;
    pending.current = true;
    setBusy(true);
    const startedErrorVersion = ++errorVersion.current;
    try {
      const incoming = await api<NotificationSnapshot>(command, args);
      if (mounted.current) {
        accept(incoming);
        if (errorVersion.current === startedErrorVersion) {
          setError(null);
          clearNotificationError();
        }
      }
    } catch (failure) {
      if (mounted.current && errorVersion.current === startedErrorVersion)
        setError(`Could not update notifications: ${errorMessage(failure)}`);
    } finally {
      pending.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  return {
    snapshot,
    error,
    busy,
    unread: unreadNotificationCount(snapshot),
    retry: () => setAttempt((value) => value + 1),
    markRead: (ids: string[]) =>
      void mutate("mark_notifications_read", { ids }),
    dismiss: (id: string) => void mutate("dismiss_notification", { id }),
    clearRead: () => void mutate("clear_read_notifications"),
  };
}

type Notifications = ReturnType<typeof useNotifications>;
export default function NotificationCenter({
  trigger,
  notifications,
  onClose,
}: {
  trigger: RefObject<HTMLButtonElement | null>;
  notifications: Notifications;
  onClose: () => void;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [position, setPosition] = useState({ left: 8, top: 8 });
  const { snapshot, error, busy, unread, retry, markRead, dismiss, clearRead } =
    notifications;
  const items = snapshot?.items ?? [];
  const close = useCallback(
    (restoreFocus: boolean) => {
      onClose();
      if (restoreFocus)
        requestAnimationFrame(() =>
          trigger.current?.focus({ preventScroll: true }),
        );
    },
    [onClose, trigger],
  );
  useLayoutEffect(() => {
    if (!trigger.current || !panel.current) return;
    const anchor = trigger.current.getBoundingClientRect();
    const bounds = panel.current.getBoundingClientRect();
    setPosition({
      left: Math.max(
        8,
        Math.min(anchor.right - bounds.width, innerWidth - bounds.width - 8),
      ),
      top: Math.max(
        8,
        Math.min(anchor.bottom + 6, innerHeight - bounds.height - 8),
      ),
    });
  }, [trigger, snapshot, error]);
  useLayoutEffect(() => {
    closeButton.current?.focus({ preventScroll: true });
  }, []);
  // A successful dismissal can remove the focused row; keep keyboard focus in
  // the overlay rather than leaving it on the document body.
  useLayoutEffect(() => {
    if (document.activeElement === document.body)
      closeButton.current?.focus({ preventScroll: true });
  }, [snapshot, busy]);
  useEffect(() => {
    const outside = (event: Event) => {
      const target = event.target as Node;
      if (
        !panel.current?.contains(target) &&
        !trigger.current?.contains(target)
      )
        close(false);
    };
    const dismissPanel = () => close(false);
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", dismissPanel);
    window.addEventListener("blur", dismissPanel);
    window.visualViewport?.addEventListener("resize", dismissPanel);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", dismissPanel);
      window.removeEventListener("blur", dismissPanel);
      window.visualViewport?.removeEventListener("resize", dismissPanel);
    };
  }, [close, trigger]);
  return createPortal(
    <div
      ref={panel}
      className="menu notification-center"
      role="dialog"
      aria-label="Notifications"
      style={position}
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          close(true);
        }
        if (event.key !== "Tab") return;
        const focusable = [
          ...event.currentTarget.querySelectorAll<HTMLElement>(
            'button:not(:disabled), [tabindex="0"]',
          ),
        ];
        const index = focusable.indexOf(document.activeElement as HTMLElement);
        if (event.shiftKey ? index <= 0 : index === focusable.length - 1) {
          event.preventDefault();
          focusable[event.shiftKey ? focusable.length - 1 : 0]?.focus();
        }
        event.stopPropagation();
      }}
    >
      <div className="notification-center-heading">
        <div>
          <strong>Notifications</strong>
          <span>
            {!snapshot
              ? "Loading…"
              : unread
                ? `${unread} unread`
                : "All caught up"}
          </span>
        </div>
        <button
          ref={closeButton}
          type="button"
          className="notification-icon-button"
          aria-label="Close notifications"
          onClick={() => close(true)}
        >
          <X size={15} aria-hidden="true" />
        </button>
      </div>
      {error && (
        <div className="notification-center-error" role="status">
          <span>{error}</span>
          <button type="button" disabled={busy} onClick={retry}>
            Retry
          </button>
        </div>
      )}
      <div
        className="notification-center-list"
        role="region"
        aria-label="Notification list"
        tabIndex={0}
        aria-busy={busy || undefined}
      >
        {!items.length && (
          <div className="notification-center-empty">
            <Bell size={24} aria-hidden="true" />
            <strong>
              {snapshot
                ? "No notifications yet"
                : error
                  ? "Notifications unavailable"
                  : "Loading notifications…"}
            </strong>
            <span>Agent updates will appear here.</span>
          </div>
        )}
        {items.map((item) => (
          <article
            className="notification-item"
            key={item.id}
            data-unread={!item.read || undefined}
          >
            <span
              className="notification-kind"
              aria-label={
                item.kind === "attention" ? "Needs attention" : "Finished"
              }
            >
              {item.kind === "attention" ? (
                <CircleAlert size={16} aria-hidden="true" />
              ) : (
                <CircleCheck size={16} aria-hidden="true" />
              )}
            </span>
            <div className="notification-item-content">
              <button
                type="button"
                className="notification-item-open"
                disabled={busy}
                aria-label={`${item.title}${item.read ? "" : ", unread"}`}
                onClick={() => {
                  if (!item.read) markRead([item.id]);
                }}
              >
                <strong>{item.title}</strong>
                <span>{item.body}</span>
              </button>
              <div className="notification-item-meta">
                <time
                  dateTime={new Date(item.createdAt).toISOString()}
                  title={new Date(item.createdAt).toLocaleString()}
                >
                  {new Date(item.createdAt).toLocaleString(undefined, {
                    month: "short",
                    day: "numeric",
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </time>
                {!item.read && (
                  <span className="notification-unread">Unread</span>
                )}
              </div>
              <div className="notification-item-actions">
                {!item.read && (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => markRead([item.id])}
                  >
                    <Check size={13} aria-hidden="true" />
                    Mark as read
                  </button>
                )}
                <button
                  type="button"
                  disabled={busy}
                  aria-label={`Dismiss ${item.title}`}
                  onClick={() => dismiss(item.id)}
                >
                  <X size={13} aria-hidden="true" />
                  Dismiss
                </button>
              </div>
            </div>
          </article>
        ))}
      </div>
      <div className="notification-center-footer">
        <button
          type="button"
          disabled={busy || !items.some((item) => item.read)}
          onClick={clearRead}
        >
          <Trash2 size={14} aria-hidden="true" />
          Clear read notifications
        </button>
      </div>
    </div>,
    document.body,
  );
}
