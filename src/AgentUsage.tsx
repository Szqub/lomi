import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { CircleAlert, Gauge, RefreshCw } from "lucide-react";
import { api, native } from "./api";
import {
  agentUsageTargetBatches,
  agentUsageSignature,
  agentUsageTargets,
  type AgentUsageEntry,
  type AgentUsageTarget,
  type AgentUsageWindow,
  worstRemainingWindow,
  validRemainingPercent,
} from "./agent-usage";
import { cliNames } from "./cli-agents";
import type { TerminalContext } from "./terminal-runtime";
import "./agent-usage.css";

interface UsageReply {
  entries: AgentUsageEntry[];
}

interface InFlightRequest {
  signature: string;
  epoch: number;
  promise: Promise<void>;
}

interface PendingRefresh {
  requested: boolean;
  force: boolean;
}

const staleAfterMs = 45_000;

function sameProcess(
  left: { id: string; process: { cli: string; pid: number } },
  right: { id: string; process: { cli: string; pid: number } },
) {
  return (
    left.id === right.id &&
    left.process.cli === right.process.cli &&
    left.process.pid === right.process.pid
  );
}

function formatPercent(value: number) {
  return `${new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 1,
    minimumFractionDigits: value > 0 && value < 1 ? 1 : 0,
  }).format(value)}%`;
}

function formatQuantity(value: number | null, unit: string | null) {
  if (value === null || !Number.isFinite(value)) return null;
  const amount = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 1,
  }).format(value);
  return unit ? `${amount} ${unit}` : amount;
}

function formatReset(timestamp: number | null) {
  if (timestamp === null || !Number.isFinite(timestamp)) return null;
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

function relativeUpdate(timestamp: number | null, now: number) {
  if (timestamp === null || !Number.isFinite(timestamp)) return null;
  const age = Math.max(0, now - timestamp);
  const seconds = Math.floor(age / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hr ago`;
  return `${Math.floor(hours / 24)} days ago`;
}

function hasUsageValues(entry: AgentUsageEntry | null) {
  return Boolean(
    entry?.windows.some(
      (window) =>
        validRemainingPercent(window.remainingPercent) ||
        window.used !== null ||
        window.limit !== null,
    ),
  );
}

function isStale(
  entry: AgentUsageEntry | null,
  requestFailed: boolean,
  now: number,
) {
  if (!entry || !hasUsageValues(entry)) return false;
  return (
    requestFailed ||
    entry.status !== "ready" ||
    entry.updatedAt === null ||
    !Number.isFinite(entry.updatedAt) ||
    now - entry.updatedAt > staleAfterMs
  );
}

function sourceName(source: string | null) {
  if (!source) return null;
  const normalized = source
    .trim()
    .toLowerCase()
    .replaceAll("_", " ")
    .replaceAll("-", " ");
  const labels: Record<string, string> = {
    account: "Account",
    "account usage": "Account",
    subscription: "Subscription",
    oauth: "Connected account",
    "connected account": "Connected account",
  };
  return labels[normalized] ?? "Account usage";
}

function entryStatus(
  entry: AgentUsageEntry | null,
  failed: boolean,
  now: number,
) {
  if (!entry) return failed ? "Refresh failed" : "Checking";
  if (failed)
    return hasUsageValues(entry)
      ? "Refresh failed · showing saved values"
      : "Refresh failed";
  switch (entry.status) {
    case "ready": {
      if (!hasUsageValues(entry))
        return entry.updatedAt === null
          ? "Not checked yet"
          : "No quotas available";
      const stale =
        entry.updatedAt === null ||
        !Number.isFinite(entry.updatedAt) ||
        now - entry.updatedAt > staleAfterMs;
      return stale ? "Out of date · showing saved values" : "Current";
    }
    case "unauthenticated":
      return "Account not connected";
    case "unsupported":
      return "Usage unavailable";
    case "error":
      return hasUsageValues(entry)
        ? "Could not refresh · showing saved values"
        : "Could not refresh usage";
    case "rate-limited":
      return hasUsageValues(entry)
        ? "Rate limited · showing saved values"
        : "Rate limited";
  }
}

function entryDescription(entry: AgentUsageEntry | null) {
  if (!entry) return "Checking this agent's account usage.";
  switch (entry.status) {
    case "ready":
      return entry.windows.length
        ? ""
        : "No quota windows are currently available.";
    case "unauthenticated":
      return "Sign in to this CLI account to view usage.";
    case "unsupported":
      return "Account usage is unavailable in this CLI mode.";
    case "error":
      return hasUsageValues(entry)
        ? "The latest check failed. The last successful values are shown below."
        : "The account usage check failed.";
    case "rate-limited":
      return hasUsageValues(entry)
        ? "The account limited this check. The last successful values are shown below."
        : "The account is temporarily limiting usage checks.";
  }
}

function safeMessage(message: string | null) {
  if (!message) return null;
  const bounded = message
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 240);
  return bounded || null;
}

function windowTone(window: AgentUsageWindow) {
  if (!validRemainingPercent(window.remainingPercent)) return "unknown";
  if (window.remainingPercent <= 10) return "critical";
  if (window.remainingPercent <= 25) return "warning";
  return "normal";
}

export function useAgentUsage() {
  const contexts = useRef<Record<string, TerminalContext>>({});
  const signature = useRef(agentUsageSignature([]));
  const epoch = useRef(0);
  const inFlight = useRef<InFlightRequest | null>(null);
  const pending = useRef<PendingRefresh>({ requested: false, force: false });
  const alive = useRef(false);
  const refreshRef = useRef<(force?: boolean) => Promise<void>>(() =>
    Promise.resolve(),
  );
  const [targets, setTargets] = useState<AgentUsageTarget[]>([]);
  const [entries, setEntries] = useState<AgentUsageEntry[]>([]);
  const [checking, setChecking] = useState(false);
  const [requestFailed, setRequestFailed] = useState(false);
  const [now, setNow] = useState(Date.now());

  const refresh = useCallback((force = false): Promise<void> => {
    const currentTargets = agentUsageTargets(contexts.current);
    if (!native || currentTargets.length === 0) {
      setChecking(false);
      return Promise.resolve();
    }

    const currentSignature = agentUsageSignature(currentTargets);
    const currentEpoch = epoch.current;
    if (inFlight.current) {
      if (inFlight.current.epoch !== currentEpoch || force) {
        pending.current.requested = true;
        pending.current.force ||= force;
      }
      return inFlight.current.promise;
    }

    setChecking(true);
    const request: InFlightRequest = {
      signature: currentSignature,
      epoch: currentEpoch,
      promise: Promise.resolve(),
    };
    request.promise = (async () => {
      try {
        const received: AgentUsageEntry[] = [];
        for (const batch of agentUsageTargetBatches(currentTargets)) {
          const result = await api<UsageReply>("inspect_cli_usage", {
            targets: batch,
            force,
          });
          if (
            !alive.current ||
            request.epoch !== epoch.current ||
            request.signature !== signature.current
          )
            return;
          received.push(...result.entries);
        }

        const latestTargets = agentUsageTargets(contexts.current);
        const accepted = received.filter((entry) =>
          latestTargets.some((target) => sameProcess(target, entry)),
        );
        setEntries(accepted);
        setRequestFailed(false);
        setNow(Date.now());
      } catch {
        if (
          alive.current &&
          request.epoch === epoch.current &&
          request.signature === signature.current
        ) {
          setRequestFailed(true);
          setNow(Date.now());
        }
      } finally {
        if (inFlight.current === request) inFlight.current = null;
        if (!alive.current) return;
        const next = pending.current;
        pending.current = { requested: false, force: false };
        if (next.requested) {
          void refreshRef.current(next.force);
        } else {
          setChecking(false);
        }
      }
    })();
    inFlight.current = request;
    return request.promise;
  }, []);
  refreshRef.current = refresh;

  const observe = useCallback((next: Record<string, TerminalContext>) => {
    contexts.current = next;
    const nextTargets = agentUsageTargets(next);
    const nextSignature = agentUsageSignature(nextTargets);
    if (nextSignature === signature.current) return;

    signature.current = nextSignature;
    epoch.current += 1;
    setTargets(nextTargets);
    setEntries((current) =>
      current.filter((entry) =>
        nextTargets.some((target) => sameProcess(target, entry)),
      ),
    );
    setRequestFailed(false);
    setNow(Date.now());

    if (inFlight.current) {
      pending.current.requested = true;
      pending.current.force = false;
    } else if (nextTargets.length) {
      void refreshRef.current(false);
    } else {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    const focused = () => {
      if (agentUsageTargets(contexts.current).length)
        void refreshRef.current(false);
    };
    focused();
    window.addEventListener("focus", focused);
    return () => {
      alive.current = false;
      epoch.current += 1;
      window.removeEventListener("focus", focused);
    };
  }, []);

  useEffect(() => {
    if (!targets.length) return;
    const timer = window.setInterval(() => {
      void refreshRef.current(false);
    }, 15_000);
    return () => window.clearInterval(timer);
  }, [targets.length]);

  const bar: ReactNode = (
    <AgentUsageBar
      targets={targets}
      entries={entries}
      checking={checking}
      requestFailed={requestFailed}
      now={now}
      onOpen={() => void refreshRef.current(false)}
      onRefresh={() => void refreshRef.current(true)}
    />
  );

  return { observe, bar };
}

function AgentUsageBar({
  targets,
  entries,
  checking,
  requestFailed,
  now,
  onOpen,
  onRefresh,
}: {
  targets: AgentUsageTarget[];
  entries: AgentUsageEntry[];
  checking: boolean;
  requestFailed: boolean;
  now: number;
  onOpen: () => void;
  onRefresh: () => void;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const agentList = useRef<HTMLDivElement>(null);
  const refreshButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 0, top: 0 });

  const entryFor = (target: AgentUsageTarget) =>
    entries.find((entry) => sameProcess(target, entry)) ?? null;
  const worst = worstRemainingWindow(entries);
  const count = targets.length;
  const worstEntryStale = worst
    ? isStale(worst.entry, requestFailed, now)
    : false;
  const displayPercent = worst
    ? formatPercent(worst.window.remainingPercent!)
    : "—";
  const unavailableEntry = targets
    .map(entryFor)
    .find((entry) => entry && entry.status !== "ready");
  const unavailableReason = requestFailed
    ? "The latest usage check failed."
    : checking
      ? "Usage is being checked."
      : unavailableEntry?.status === "unauthenticated"
        ? "The CLI account is not connected."
        : unavailableEntry?.status === "unsupported"
          ? "Usage is unavailable in the current CLI mode."
          : unavailableEntry?.status === "rate-limited"
            ? "The account is limiting usage checks."
            : unavailableEntry?.status === "error"
              ? "The latest usage check failed."
              : "No quota data is available yet.";
  const title = worst
    ? `${cliNames[worst.entry.process.cli]} · ${worst.window.label}: ${displayPercent} remaining${worstEntryStale ? " · stale data" : ""}. Open account usage details.`
    : `${unavailableReason} Open account usage details for ${count} active CLI agent${count === 1 ? "" : "s"}.`;
  const label = worst
    ? `CLI account usage, ${displayPercent} remaining${count > 1 ? ` across ${count} agents` : ""}${worstEntryStale ? ", stale data" : ""}`
    : `CLI account usage unavailable${count > 1 ? `, ${count} agents` : ""}: ${unavailableReason}`;

  const close = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    if (restoreFocus)
      requestAnimationFrame(() =>
        trigger.current?.focus({ preventScroll: true }),
      );
  }, []);

  const show = useCallback(() => {
    setOpen(true);
    onOpen();
  }, [onOpen]);

  useLayoutEffect(() => {
    if (!open || !trigger.current || !menu.current) return;
    const triggerBounds = trigger.current.getBoundingClientRect();
    const menuBounds = menu.current.getBoundingClientRect();
    const left = Math.max(
      8,
      Math.min(
        triggerBounds.right - menuBounds.width,
        innerWidth - menuBounds.width - 8,
      ),
    );
    const below = triggerBounds.bottom + 6;
    const top =
      below + menuBounds.height <= innerHeight - 8
        ? below
        : Math.max(8, triggerBounds.top - menuBounds.height - 6);
    setPosition({ left, top });
  }, [entries, open, targets]);

  useLayoutEffect(() => {
    if (!open) return;
    refreshButton.current?.focus({ preventScroll: true });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      const target = event.target as Node;
      if (trigger.current?.contains(target) || menu.current?.contains(target))
        return;
      close(false);
    };
    const dismiss = () => close(false);
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", dismiss);
    window.addEventListener("blur", dismiss);
    window.visualViewport?.addEventListener("resize", dismiss);
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("blur", dismiss);
      window.visualViewport?.removeEventListener("resize", dismiss);
    };
  }, [close, open]);

  useLayoutEffect(() => {
    if (!count && open) setOpen(false);
  }, [count, open]);

  if (!count) return null;

  const tone = worst
    ? windowTone(worst.window)
    : requestFailed
      ? "stale"
      : "unknown";

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="agent-usage-trigger"
        data-tone={tone}
        title={title}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => (open ? close(false) : show())}
        onContextMenu={(event) => {
          event.preventDefault();
          show();
        }}
        onKeyDown={(event) => {
          if (
            event.key === "ArrowDown" ||
            event.key === "ContextMenu" ||
            (event.shiftKey && event.key === "F10")
          ) {
            event.preventDefault();
            event.stopPropagation();
            show();
          }
        }}
      >
        <Gauge size={13} strokeWidth={1.8} aria-hidden="true" />
        <span className="agent-usage-trigger-value">{displayPercent}</span>
        {count > 1 && (
          <span className="agent-usage-count" aria-hidden="true">
            {count}
          </span>
        )}
        {worstEntryStale && (
          <span className="agent-usage-stale-dot" aria-hidden="true" />
        )}
      </button>
      {open &&
        createPortal(
          <div
            ref={menu}
            className="menu agent-usage-menu"
            role="menu"
            aria-label="CLI account usage"
            style={position}
            onContextMenu={(event) => event.preventDefault()}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                close(true);
                return;
              }
              if (event.key === "Tab") {
                event.preventDefault();
                event.stopPropagation();
                if (event.target === agentList.current)
                  refreshButton.current?.focus({ preventScroll: true });
                else agentList.current?.focus({ preventScroll: true });
                return;
              }
              if (
                agentList.current?.contains(event.target as Node) &&
                [
                  "ArrowUp",
                  "ArrowDown",
                  "Home",
                  "End",
                  "PageUp",
                  "PageDown",
                  " ",
                ].includes(event.key)
              )
                return;
              if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key))
                return;
              event.preventDefault();
              const items = [
                ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
                  'button[role="menuitem"]:not(:disabled)',
                ),
              ];
              const index = items.indexOf(
                document.activeElement as HTMLButtonElement,
              );
              items[
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? items.length - 1
                    : (index +
                        (event.key === "ArrowDown" ? 1 : -1) +
                        items.length) %
                      items.length
              ]?.focus();
            }}
          >
            <div className="agent-usage-menu-heading" role="presentation">
              <strong>Account usage</strong>
              <span>
                {count} active CLI agent{count === 1 ? "" : "s"}
              </span>
            </div>
            <div
              ref={agentList}
              className="agent-usage-list"
              role="region"
              aria-label="Usage details by agent"
              tabIndex={0}
            >
              {targets.map((target, index) => {
                const entry = entryFor(target);
                const stale = isStale(entry, requestFailed, now);
                const updated = relativeUpdate(entry?.updatedAt ?? null, now);
                const statusText = entryStatus(entry, requestFailed, now);
                const message = safeMessage(entry?.message ?? null);
                const description = message ? "" : entryDescription(entry);
                const freshness = updated
                  ? `${stale ? "Last successful check" : "Updated"} · ${updated}`
                  : "No successful usage check";
                return (
                  <section
                    className="agent-usage-agent"
                    role="group"
                    aria-label={`${cliNames[target.process.cli]} usage ${index + 1}`}
                    key={`${target.id}:${target.process.cli}:${target.process.pid}`}
                  >
                    <div className="agent-usage-agent-heading">
                      <strong>{cliNames[target.process.cli]}</strong>
                      <span
                        className="agent-usage-status"
                        data-stale={stale || undefined}
                      >
                        {stale && <CircleAlert size={12} aria-hidden="true" />}
                        {statusText}
                      </span>
                    </div>
                    {description && (
                      <p className="agent-usage-description">{description}</p>
                    )}
                    {message && (
                      <p className="agent-usage-description">{message}</p>
                    )}
                    {entry?.windows.length ? (
                      <div className="agent-usage-windows">
                        {entry.windows.map((window, windowIndex) => (
                          <UsageWindow
                            key={`${window.label}:${windowIndex}`}
                            window={window}
                          />
                        ))}
                      </div>
                    ) : !description && !message ? (
                      <p className="agent-usage-no-windows">
                        {entry
                          ? "No quota windows to show."
                          : "Usage data is loading."}
                      </p>
                    ) : null}
                    <div className="agent-usage-meta">
                      <span>{freshness}</span>
                      {entry?.source && (
                        <span>Source · {sourceName(entry.source)}</span>
                      )}
                    </div>
                    {entry?.retryAt !== null &&
                      entry?.retryAt !== undefined && (
                        <p className="agent-usage-retry">
                          Try again after{" "}
                          {formatReset(entry.retryAt) ?? "the next check"}.
                        </p>
                      )}
                  </section>
                );
              })}
            </div>
            <div role="separator" className="menu-divider" />
            <button
              ref={refreshButton}
              type="button"
              role="menuitem"
              tabIndex={-1}
              className="menu-item agent-usage-refresh"
              aria-busy={checking || undefined}
              onClick={() => {
                onRefresh();
                refreshButton.current?.focus({ preventScroll: true });
              }}
            >
              <RefreshCw size={14} aria-hidden="true" />
              <span>{checking ? "Refreshing…" : "Refresh"}</span>
            </button>
          </div>,
          document.body,
        )}
    </>
  );
}

function UsageWindow({ window }: { window: AgentUsageWindow }) {
  const remaining = validRemainingPercent(window.remainingPercent)
    ? window.remainingPercent
    : null;
  const used = formatQuantity(window.used, window.unit);
  const limit = formatQuantity(window.limit, window.unit);
  const reset = formatReset(window.resetsAt);
  const amount =
    used && limit
      ? `${used} / ${limit}`
      : used
        ? `${used} used`
        : limit
          ? `Limit ${limit}`
          : null;

  return (
    <div className="agent-usage-window" data-tone={windowTone(window)}>
      <div className="agent-usage-window-heading">
        <strong>{window.label || "Quota window"}</strong>
        <span>
          {remaining === null
            ? "Quota unknown"
            : `${formatPercent(remaining)} remaining`}
        </span>
      </div>
      {remaining !== null && (
        <progress
          max={100}
          value={100 - remaining}
          aria-label={`${window.label || "Quota window"} usage: ${formatPercent(100 - remaining)} used`}
        />
      )}
      <div className="agent-usage-window-meta">
        {amount && <span>{amount}</span>}
        <span>{reset ? `Resets ${reset}` : "Reset time unavailable"}</span>
      </div>
    </div>
  );
}
