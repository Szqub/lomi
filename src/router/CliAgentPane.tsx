import { useSyncExternalStore } from "react";
import type { CliAgentTab } from "../model";
import { IconButton } from "../ui";
import { FileCode, History, MessageSquare, Send, Terminal, X } from "../icons";
import { SettingsNotice } from "../settings-ui";
import { getRunView } from "./run-runtime";
import { isRunning } from "./model";
import RunDataGrant from "./RunDataGrant";
import CodingApproval from "./CodingApproval";
import NativeApproval from "./NativeApproval";
import NativeHandoff from "./NativeHandoff";
import GatewayCliPane from "./GatewayCliPane";
import { runStateLabels } from "./presentation";
import "./router.css";
import "./router-launch.css";

export default function CliAgentPane({
  tab,
  onFocus,
  onClose,
}: {
  tab: CliAgentTab;
  onFocus: () => void;
  onClose: () => void;
}) {
  const runtime = getRunView(tab.runId);
  const state = useSyncExternalStore(runtime.subscribe, runtime.getSnapshot);
  const run = state.snapshot?.runs.find((item) => item.id === tab.runId);
  const running = !!run && isRunning(run.state);
  const recovery =
    !!run &&
    (run.state === "recovery_required" ||
      (run.inputs.length > 0 &&
        ["waiting_for_capacity", "stopped", "paused"].includes(run.state)));
  const router = state.snapshot?.routers.find(
    (item) => item.id === run?.routerId,
  );
  const capability = state.snapshot?.capabilities.find(
    (item) => item.cli === router?.cli,
  );
  const coding = run?.executionMode === "coding";
  const native = run?.executionMode === "native";
  const canSend = native
    ? capability?.nativeTurns
    : coding
      ? capability?.codingTurns
      : capability?.managedTurns;
  const textLimit = coding ? 60 * 1024 : 64 * 1024;
  const textTooLarge = new TextEncoder().encode(state.draft).length > textLimit;
  const canAcknowledge =
    (coding || native) &&
    !!run?.inputs.length &&
    ["recovery_required", "stopped", "paused"].includes(run.state);
  const command = runtime.command.bind(runtime);
  async function send(handoff: boolean) {
    if (!run || !canSend || (!handoff && (!state.draft.trim() || textTooLarge)))
      return;
    const next = await command("cli_run_send", {
      request: {
        requestId: crypto.randomUUID(),
        runId: run.id,
        expectedRevision: run.revision,
        text: handoff ? "" : state.draft,
        handoff,
      },
    });
    if (next && !handoff) runtime.setDraft("");
  }
  if (run?.executionMode === "gateway") {
    return (
      <GatewayCliPane
        tab={tab}
        run={run}
        snapshot={state.snapshot!}
        busy={state.busy}
        error={state.error}
        command={command}
        onFocus={onFocus}
        onClose={onClose}
      />
    );
  }
  return (
    <section
      className="cli-agent-pane router-saved-pane"
      onPointerDown={onFocus}
      onFocusCapture={onFocus}
      aria-label={tab.customTitle ?? tab.title}
    >
      <header className="cli-agent-header">
        <div className="cli-agent-heading">
          <strong>{tab.customTitle ?? run?.title ?? tab.title}</strong>
          {run && (
            <span className="router-state" data-state={run.state}>
              {runStateLabels[run.state]}
            </span>
          )}
        </div>
        <IconButton title="Close CLI Agent view" onClick={onClose}>
          <X size={14} />
        </IconButton>
      </header>
      <div className="cli-agent-content router-pane-content">
        {state.error && (
          <SettingsNotice tone="error">{state.error}</SettingsNotice>
        )}
        {!state.loaded && !state.error && (
          <p role="status">Loading saved CLI run…</p>
        )}
        {state.loaded && !run && (
          <SettingsNotice>
            Saved run unavailable. Open another run from Agents → Open router.
          </SettingsNotice>
        )}
        {run && (
          <>
            <div className="router-pane-meta">
              {coding ? (
                <FileCode size={14} aria-hidden="true" />
              ) : native ? (
                <Terminal size={14} aria-hidden="true" />
              ) : (
                <MessageSquare size={14} aria-hidden="true" />
              )}
              <span>{capability?.name ?? router?.cli ?? "CLI Agent"}</span>
              <span title={run.model ?? undefined}>
                {run.model ?? "No model"}
              </span>
              <span>
                {coding ? "Edit files" : native ? "CLI account" : "Chat"}
              </span>
              {run.reasoningEffort && <span>{run.reasoningEffort}</span>}
            </div>
            <div className="router-run-controls router-pane-toolbar">
              <label>
                <span>Account</span>
                <select
                  aria-label="Account for next attempt"
                  value={run.pinnedProfileId ?? ""}
                  disabled={state.busy || running}
                  onChange={(event) =>
                    void command("cli_run_pin", {
                      runId: run.id,
                      profileId: event.target.value || null,
                    })
                  }
                >
                  <option value="">Router order</option>
                  {run.allowedProfileIds.map((id) => (
                    <option key={id} value={id}>
                      {state.snapshot?.profiles.find(
                        (profile) => profile.id === id,
                      )?.label ?? "Unavailable account"}
                    </option>
                  ))}
                </select>
              </label>
              {state.snapshot && (
                <RunDataGrant
                  snapshot={state.snapshot}
                  run={run}
                  disabled={state.busy || running}
                  busy={state.busy}
                  error={state.error}
                  command={command}
                />
              )}
              {native && state.snapshot && (
                <NativeHandoff
                  key={run.id}
                  snapshot={state.snapshot}
                  run={run}
                  disabled={state.busy || running}
                />
              )}
            </div>
            {!!run.attempts.length && (
              <details className="router-disclosure router-pane-history">
                <summary>
                  <History size={14} aria-hidden="true" /> Account history{" "}
                  <span>{run.attempts.length}</span>
                </summary>
                <ol>
                  {run.attempts.map((attempt) => (
                    <li key={attempt.id}>
                      <strong>
                        {state.snapshot?.profiles.find(
                          (profile) => profile.id === attempt.profileId,
                        )?.label ?? "Unavailable account"}
                      </strong>
                      <span>
                        {attempt.state.replaceAll("_", " ")}
                        {attempt.reason && ` · ${attempt.reason}`}
                      </span>
                    </li>
                  ))}
                </ol>
              </details>
            )}
            {run.statusMessage && (
              <SettingsNotice>{run.statusMessage}</SettingsNotice>
            )}
            <section
              className="router-pane-transcript"
              aria-label="Conversation"
            >
              {run.inputs.map((input) => (
                <article key={input.id} className="cli-agent-turn">
                  <h3 className="router-pane-actor">
                    <MessageSquare size={12} aria-hidden="true" />
                    You
                  </h3>
                  <pre>{input.text}</pre>
                  {run.turns
                    ?.filter((turn) => turn.inputId === input.id)
                    .map((turn) => (
                      <div key={turn.attemptId}>
                        <h3 className="router-pane-actor">
                          <Terminal size={12} aria-hidden="true" />
                          {turn.state === "completed"
                            ? "Agent"
                            : turn.state === "rejected"
                              ? "Rejected before response"
                              : "Uncertain response"}
                        </h3>
                        {turn.state !== "completed" &&
                          turn.state !== "rejected" && (
                            <p className="settings-help">
                              {native
                                ? "Files may have changed. Review the original account before resuming."
                                : coding
                                  ? "Recover uncertain changes before continuing."
                                  : "This partial response is retained as context."}
                            </p>
                          )}
                        <pre>{turn.text || "No saved response text."}</pre>
                      </div>
                    ))}
                </article>
              ))}
              {run.legacyOutput && (
                <article className="cli-agent-turn">
                  <h3>Earlier saved output</h3>
                  <p className="settings-help">
                    This output has no recorded message boundaries.
                  </p>
                  <pre>{run.legacyOutput}</pre>
                </article>
              )}
              {(running || (!run.turns?.length && !run.legacyOutput)) &&
                run.output && (
                  <article className="cli-agent-turn">
                    <h3>{running ? "Live output" : "Saved output"}</h3>
                    <pre>{run.output}</pre>
                  </article>
                )}
            </section>
            {coding && <CodingApproval runId={run.id} />}
            {native && <NativeApproval runId={run.id} />}
            {canAcknowledge && (
              <button
                className="button"
                disabled={state.busy}
                onClick={() =>
                  void command("cli_run_acknowledge_coding_completion", {
                    request: {
                      requestId: crypto.randomUUID(),
                      runId: run.id,
                      expectedRunRevision: run.revision,
                    },
                  })
                }
              >
                Acknowledge recorded completion
              </button>
            )}
            {running ? (
              <button
                className="button"
                disabled={state.busy}
                onClick={() => void command("cli_run_stop", { runId: run.id })}
              >
                Stop
              </button>
            ) : recovery ? (
              <>
                <SettingsNotice>
                  {native
                    ? "Review uncertain file changes in the original account before resuming."
                    : coding
                      ? "Recover uncertain changes before continuing with saved history and tool results."
                      : "Review partial output before a new attempt with approved saved context."}
                </SettingsNotice>
                <button
                  className="button button-primary"
                  disabled={state.busy || !canSend}
                  onClick={() => void send(true)}
                >
                  {native ? "Resume checkpointed task" : "Continue saved task"}
                </button>
                {native && (
                  <button
                    className="button"
                    disabled={state.busy}
                    onClick={() =>
                      void command("cli_run_open_native_recovery", {
                        runId: run.id,
                        expectedRevision: run.revision,
                      })
                    }
                  >
                    Review in native account terminal
                  </button>
                )}
              </>
            ) : (
              <form
                className="router-run-form router-pane-composer"
                onSubmit={(event) => {
                  event.preventDefault();
                  void send(false);
                }}
              >
                <label>
                  <span className="router-launch-visually-hidden">Message</span>
                  <textarea
                    value={state.draft}
                    maxLength={textLimit}
                    disabled={state.busy || !canSend}
                    placeholder="What would you like to do?"
                    onChange={(event) => runtime.setDraft(event.target.value)}
                  />
                </label>
                {textTooLarge && (
                  <p role="alert">
                    Message exceeds the {coding ? "60" : "64"} KiB UTF-8 limit.
                  </p>
                )}
                <button
                  className="button button-primary"
                  disabled={
                    state.busy ||
                    !state.draft.trim() ||
                    textTooLarge ||
                    !canSend
                  }
                >
                  <Send size={14} aria-hidden="true" /> Send
                </button>
              </form>
            )}
          </>
        )}
      </div>
    </section>
  );
}
