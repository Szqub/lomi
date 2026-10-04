import { useSyncExternalStore } from "react";
import type { CliAgentTab } from "../model";
import { IconButton } from "../ui";
import { X } from "../icons";
import { SettingsNotice } from "../settings-ui";
import { getRunView } from "./run-runtime";
import { isRunning } from "./model";
import RunDataGrant from "./RunDataGrant";
import CodingApproval from "./CodingApproval";
import NativeApproval from "./NativeApproval";
import NativeHandoff from "./NativeHandoff";
import GatewayCliPane from "./GatewayCliPane";
import "./router.css";

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
      className="cli-agent-pane"
      onPointerDown={onFocus}
      onFocusCapture={onFocus}
      aria-label={tab.customTitle ?? tab.title}
    >
      <header className="cli-agent-header">
        <strong>{tab.customTitle ?? run?.title ?? tab.title}</strong>
        <IconButton title="Close CLI Agent view" onClick={onClose}>
          <X size={14} />
        </IconButton>
      </header>
      <div className="cli-agent-content">
        {state.error && (
          <SettingsNotice tone="error">{state.error}</SettingsNotice>
        )}
        {!state.loaded && !state.error && (
          <p role="status">Loading saved CLI run…</p>
        )}
        {state.loaded && !run && (
          <SettingsNotice>
            This saved run is no longer available. Close this view or open
            another run from Agents → Open router.
          </SettingsNotice>
        )}
        {run && (
          <>
            <p className="settings-help">
              {capability?.name ?? router?.cli ?? "CLI Agent"} ·{" "}
              {run.model ?? "No model"}
              {run.reasoningEffort
                ? ` · ${run.reasoningEffort} reasoning`
                : ""}{" "}
              · {run.state.replaceAll("_", " ")}
            </p>
            <p className="settings-help">
              {native
                ? "Native coding retains CLI tools, conversation state and permission policies. Permission requests appear here. The CLI executes its own file and shell operations."
                : coding
                  ? "Coding retains native conversation history. Four bounded project tools can list, read, write and apply structured patches. Every write or patch requires your explicit approval. Native shell execution is unavailable."
                  : "Retained logical text context is sent in a fresh CLI conversation. Project files and tools are unavailable."}
            </p>
            {run.statusMessage && (
              <SettingsNotice>{run.statusMessage}</SettingsNotice>
            )}
            {run.inputs.map((input) => (
              <article key={input.id} className="cli-agent-turn">
                <h3>Your message</h3>
                <pre>{input.text}</pre>
                {run.turns
                  ?.filter((turn) => turn.inputId === input.id)
                  .map((turn) => (
                    <div key={turn.attemptId}>
                      <h3>
                        {turn.state === "completed"
                          ? "CLI response"
                          : turn.state === "rejected"
                            ? "Attempt rejected before a response"
                            : "Uncertain partial response"}
                      </h3>
                      {turn.state !== "completed" &&
                        turn.state !== "rejected" && (
                          <p className="settings-help">
                            {native
                              ? "This native attempt may have changed files. Resume requires a settled checkpoint; review its exact session in the original account terminal when observations are incomplete."
                              : coding
                                ? "This attempt did not complete. Continue uses fully recorded native history. Unknown dispatches or effects require recovery before another attempt."
                                : "This attempt did not complete. Continuing the saved task explicitly approves carrying its partial text forward."}
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
                  <h3>{running ? "Live cumulative output" : "Saved output"}</h3>
                  <pre>{run.output}</pre>
                </article>
              )}
            <label>
              Account for next attempt
              <select
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
                    ? "Resume uses a settled native checkpoint in its original account. If the result or tool effects are uncertain, review the exact saved session in that account's terminal. The original task is never automatically replayed."
                    : coding
                      ? "Continue retains native history and recorded tool results. It never resends the original task. Uncertain dispatches and file changes remain blocked until recovered. Acknowledge recorded completion returns a fully completed input to idle without launching a turn."
                      : "Continue starts a fresh attempt using the saved task and approved text context. It can run on another eligible account."}
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
                className="router-run-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  void send(false);
                }}
              >
                <label>
                  Message
                  <textarea
                    value={state.draft}
                    maxLength={textLimit}
                    disabled={state.busy || !canSend}
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
                  Send
                </button>
              </form>
            )}
          </>
        )}
      </div>
    </section>
  );
}
