import { useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { api, errorMessage, getInfo } from "../api";
import type { CliAgent } from "../cli-agents";
import type { CliAgentTab } from "../model";
import { Globe, X } from "../icons";
import { IconButton } from "../ui";
import { SettingsNotice } from "../settings-ui";
import {
  closeTerminals,
  gatewayTerminal,
  terminalFor,
  type TerminalRuntime,
} from "../terminal-runtime";
import type { CliRouterSnapshot, CliRun } from "./types";
import RunDataGrant from "./RunDataGrant";
import { runStateLabels } from "./presentation";
import "./router-launch.css";
const subscribeIdle = () => () => {};

export default function GatewayCliPane({
  tab,
  run,
  snapshot,
  busy,
  error,
  command,
  onFocus,
  onClose,
}: {
  tab: CliAgentTab;
  run: CliRun;
  snapshot: CliRouterSnapshot;
  busy: boolean;
  error: string;
  command: (
    name: string,
    args: Record<string, unknown>,
  ) => Promise<CliRouterSnapshot | undefined>;
  onFocus: () => void;
  onClose: () => void;
}) {
  const [terminal, setTerminal] = useState(() => gatewayTerminal(run.id));
  const [starting, setStarting] = useState(false);
  const [failure, setFailure] = useState("");
  const live = useRef(true);
  useLayoutEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const current = useRef({ run, busy });
  current.current = { run, busy };
  const router = snapshot.routers.find((item) => item.id === run.routerId);
  const capability = snapshot.capabilities.find(
    (item) => item.cli === router?.cli,
  );
  const initial = run.generation === 0 && run.state === "idle";
  const terminalStatus = useSyncExternalStore(
    terminal?.subscribe ?? subscribeIdle,
    () => terminal?.getSnapshot().status ?? "idle",
  );
  const resumable =
    run.state === "stopped" &&
    run.generation > 0 &&
    !!capability?.nativeHistoryResume;
  const canPrepare =
    (initial && !terminal) ||
    (resumable && (!terminal || terminalStatus === "exited"));
  async function start() {
    if (
      busy ||
      starting ||
      !canPrepare ||
      !capability?.gatewayTerminal ||
      !router
    )
      return;
    const reviewed = run;
    setStarting(true);
    setFailure("");
    try {
      const info = await getInfo();
      if (
        !live.current ||
        current.current.busy ||
        current.current.run.revision !== reviewed.revision ||
        current.current.run.id !== reviewed.id
      )
        throw new Error(
          "The run changed. Review its current state before starting.",
        );
      const profile = info.profiles.find(
        (item) => item.id === reviewed.shellProfileId,
      );
      if (!profile)
        throw new Error(
          "The saved shell environment is unavailable. Create a new run with an installed environment.",
        );
      const existing = gatewayTerminal(reviewed.id);
      if (existing && resumable) {
        await api("close_terminal", { id: existing.sessionId });
        closeTerminals([existing.paneId]);
        setTerminal(undefined);
        if (!live.current || current.current.run.revision !== reviewed.revision)
          throw new Error(
            "The saved run changed before resume. Review its current state.",
          );
      }
      const runtime =
        (resumable ? undefined : existing) ??
        terminalFor(
          {
            type: "terminal",
            id: `gateway:${reviewed.id}:${crypto.randomUUID()}`,
            cwd: reviewed.cwd,
            profileId: profile.id,
          },
          profile,
          router.cli as CliAgent,
          undefined,
          undefined,
          { runId: reviewed.id, revision: reviewed.revision },
        );
      setTerminal(runtime);
    } catch (cause) {
      if (live.current) setFailure(errorMessage(cause));
    } finally {
      if (live.current) setStarting(false);
    }
  }
  async function stop() {
    setFailure("");
    const next = await command("cli_run_stop", { runId: run.id });
    if (!next) return;
    const runtime = gatewayTerminal(run.id);
    if (!runtime) return;
    try {
      await api("close_terminal", { id: runtime.sessionId });
      closeTerminals([runtime.paneId]);
      setTerminal(undefined);
    } catch (cause) {
      setFailure(errorMessage(cause));
    }
  }
  return (
    <section
      className="cli-agent-pane router-saved-pane"
      aria-label={tab.customTitle ?? tab.title}
      onPointerDown={onFocus}
      onFocusCapture={onFocus}
    >
      <header className="cli-agent-header">
        <div className="cli-agent-heading">
          <strong>{tab.customTitle ?? run.title}</strong>
          <span className="router-state" data-state={run.state}>
            {runStateLabels[run.state]}
          </span>
        </div>
        <IconButton title="Close CLI Agent view" onClick={onClose}>
          <X size={14} />
        </IconButton>
      </header>
      <div className="cli-agent-content router-gateway-content">
        {(error || failure) && (
          <SettingsNotice tone="error">{failure || error}</SettingsNotice>
        )}
        <div className="router-pane-meta">
          <Globe size={14} aria-hidden="true" />
          <span>{capability?.name ?? router?.cli}</span>
          <span title={run.model ?? undefined}>{run.model}</span>
          <span>API terminal</span>
        </div>
        {run.statusMessage && (
          <SettingsNotice>{run.statusMessage}</SettingsNotice>
        )}
        <div className="router-inline-actions router-pane-toolbar">
          <RunDataGrant
            snapshot={snapshot}
            run={run}
            disabled={busy || starting || !!terminal}
            busy={busy || starting}
            error={error}
            command={command}
          />
          {canPrepare && (
            <button
              className="button button-primary"
              disabled={busy || starting || !capability?.gatewayTerminal}
              onClick={() => void start()}
            >
              {starting
                ? "Preparing CLI…"
                : resumable
                  ? "Resume CLI"
                  : "Start CLI"}
            </button>
          )}
        </div>
        {canPrepare && (
          <p className="settings-help">
            Approved API accounts · CLI tools and permissions.
          </p>
        )}
        {!initial && !terminal && (
          <p className="settings-help">
            {resumable
              ? "Resume uses saved CLI history; no message is sent."
              : "History saved. Create a new run if safe resume is unavailable."}
          </p>
        )}
        {((terminal && terminalStatus !== "exited") ||
          ["starting", "running", "switching"].includes(run.state)) && (
          <button
            className="button"
            disabled={busy || starting}
            onClick={() => void stop()}
          >
            Stop CLI
          </button>
        )}
      </div>
      {terminal && <AttachedTerminal runtime={terminal} onFocus={onFocus} />}
    </section>
  );
}
function AttachedTerminal({
  runtime,
  onFocus,
}: {
  runtime: TerminalRuntime;
  onFocus: () => void;
}) {
  const state = useSyncExternalStore(runtime.subscribe, runtime.getSnapshot);
  const container = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    runtime.attach(container.current!);
    return () => runtime.detach();
  }, [runtime]);
  return (
    <section
      className="terminal-pane is-active"
      data-pane-id={runtime.paneId}
      aria-label={`Gateway terminal ${runtime.profile.name}`}
      onFocusCapture={onFocus}
      style={{ flex: 1, minHeight: 0 }}
    >
      {state.error && (
        <SettingsNotice tone="error">{state.error}</SettingsNotice>
      )}
      {state.status === "exited" && (
        <p role="status">CLI exited. Native history saved.</p>
      )}
      <div className="terminal-mount" ref={container} />
    </section>
  );
}
