import { cliNames, type CliAgent } from "../cli-agents";
import { useEffect, useRef, useState } from "react";
import { Modal } from "../ui";
import { SettingsNotice } from "../settings-ui";
import type { CliRun } from "./types";
import { isRunning } from "./model";
import { useRouterSnapshot } from "./useRouterSnapshot";
import RunDataGrant from "./RunDataGrant";
import CodingApproval from "./CodingApproval";
import NativeApproval from "./NativeApproval";
import NativeHandoff from "./NativeHandoff";
import {
  ArrowLeft,
  Check,
  ChevronRight,
  FileCode,
  Folder,
  Globe,
  MessageSquare,
  Play,
  Plus,
  RotateCcw,
  Terminal,
} from "../icons";
import { runStateLabels } from "./presentation";
import "./router.css";
import "./router-launch.css";
const launchModes = {
  text: {
    label: "Chat",
    hint: "Messages only · no files or tools",
    Icon: MessageSquare,
  },
  coding: {
    label: "Edit files",
    hint: "Approve each write · no shell",
    Icon: FileCode,
  },
  gateway: {
    label: "API terminal",
    hint: "API keys · CLI tools and permissions",
    Icon: Globe,
  },
  native: {
    label: "CLI account",
    hint: "CLI login · permission requests in Lomi",
    Icon: Terminal,
  },
};
export default function RouterRunDialog({
  cwd,
  shellProfileId,
  onClose,
  onOpenRun,
}: {
  cwd: string;
  shellProfileId: string;
  onClose: () => void;
  onOpenRun?: (run: CliRun) => void;
}) {
  const [selectedRun, setSelectedRun] = useState("");
  const [historyOpen, setHistoryOpen] = useState(false);
  const [step, setStep] = useState<0 | 1 | 2>(0);
  const stepHeading = useRef<HTMLHeadingElement>(null);
  const [pendingRunId, setPendingRunId] = useState<string>();
  const completedCreation = useRef<string | undefined>(undefined);
  const [routerId, setRouterId] = useState("");
  const [model, setModel] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState("");
  const [executionMode, setExecutionMode] = useState<
    "" | "text" | "coding" | "gateway" | "native"
  >("text");
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [poll, setPoll] = useState(false);
  const [removing, setRemoving] = useState<{
    id: string;
    title: string;
    revision: number;
  }>();
  const cancelRemoval = useRef<HTMLButtonElement>(null);
  const { snapshot, error, busy, command } = useRouterSnapshot(poll);
  const runs = snapshot?.runs.filter((run) => run.cwd === cwd) ?? [];
  const run = runs.find((item) => item.id === selectedRun);
  const running = !!run && isRunning(run.state);
  const shouldPoll = runs.some((item) => isRunning(item.state));
  useEffect(() => setPoll(shouldPoll), [shouldPoll]);
  const routers =
    snapshot?.routers.filter(
      (router) =>
        router.enabled &&
        snapshot.capabilities.some(
          (capability) =>
            capability.cli === router.cli &&
            (capability.canStart ||
              capability.managedTurns ||
              capability.gatewayTerminal ||
              capability.nativeTurns),
        ),
    ) ?? [];
  const activeRouter = snapshot?.routers.find(
    (item) => item.id === (run?.routerId ?? routerId),
  );
  const capability = snapshot?.capabilities.find(
    (item) => item.cli === activeRouter?.cli,
  );
  const cliName =
    capability?.name ||
    cliNames[activeRouter?.cli as CliAgent] ||
    activeRouter?.cli;
  const terminalMode =
    !!capability?.canStart &&
    !capability.managedTurns &&
    !capability.gatewayTerminal;
  const gateway = run
    ? run.executionMode === "gateway"
    : executionMode === "gateway";
  const requiresReasoning =
    activeRouter?.cli === "codex" && !terminalMode && !gateway;
  const native = run
    ? run.executionMode === "native"
    : executionMode === "native";
  const modelChoices = gateway || native ? undefined : capability?.models;
  const selectedModel = modelChoices?.find((choice) => choice.id === model);
  const reasoningChoices = selectedModel?.reasoningEfforts ?? [];
  const validModel = modelChoices ? !!selectedModel : !!model.trim();
  const validReasoning =
    !requiresReasoning ||
    (modelChoices
      ? reasoningChoices.includes(reasoningEffort)
      : !!reasoningEffort.trim());
  useEffect(() => {
    setModel("");
    setReasoningEffort("");
    setExecutionMode(
      capability?.managedTurns &&
        !capability.codingTurns &&
        !capability.gatewayTerminal
        ? "text"
        : "",
    );
  }, [
    activeRouter?.cli,
    capability?.managedTurns,
    capability?.codingTurns,
    capability?.gatewayTerminal,
    capability?.nativeTurns,
  ]);
  const recovery =
    !!run &&
    (run.state === "recovery_required" ||
      (run.inputs.length > 0 &&
        ["waiting_for_capacity", "stopped", "paused"].includes(run.state)));
  const codingAvailable =
    activeRouter?.cli === "codex" && !!capability?.codingTurns;
  const validExecution =
    (executionMode === "text" && !!capability?.managedTurns) ||
    (executionMode === "coding" && codingAvailable) ||
    (executionMode === "native" && !!capability?.nativeTurns) ||
    (executionMode === "gateway" && !!capability?.gatewayTerminal);
  const routeReady = routers.some((item) => item.id === routerId);
  const availableModes = (
    Object.keys(launchModes) as (keyof typeof launchModes)[]
  ).filter((mode) =>
    mode === "text"
      ? capability?.managedTurns
      : mode === "coding"
        ? codingAvailable
        : mode === "gateway"
          ? capability?.gatewayTerminal
          : capability?.nativeTurns,
  );
  useEffect(() => {
    if (historyOpen) return;
    if (step > 0 && !routeReady) setStep(0);
    else if (step === 2 && !terminalMode && !validExecution) setStep(1);
  }, [historyOpen, step, routeReady, terminalMode, validExecution]);
  useEffect(() => {
    if (!historyOpen) stepHeading.current?.focus();
  }, [step, historyOpen]);
  useEffect(() => {
    if (!pendingRunId || busy) return;
    const created = snapshot?.runs.find(
      (item) => item.id === pendingRunId && item.cwd === cwd,
    );
    if (!created || completedCreation.current === created.id) return;
    completedCreation.current = created.id;
    setPendingRunId(undefined);
    if (onOpenRun) onOpenRun(created);
    else {
      setSelectedRun(created.id);
      setHistoryOpen(true);
    }
  }, [pendingRunId, snapshot, busy, cwd, onOpenRun]);
  function nextStep() {
    if (busy) return;
    if (step === 0 && routeReady) setStep(terminalMode ? 2 : 1);
    else if (step === 1 && validExecution) setStep(2);
  }
  const coding = run?.executionMode === "coding";
  const canSend = native
    ? capability?.nativeTurns
    : coding
      ? capability?.codingTurns
      : capability?.managedTurns;
  const textLimit = coding ? 60 * 1024 : 64 * 1024;
  const textTooLarge = new TextEncoder().encode(text).length > textLimit;
  const canAcknowledge =
    (coding || native) &&
    !!run?.inputs.length &&
    ["recovery_required", "stopped", "paused"].includes(run.state);
  async function send(handoff: boolean) {
    if (
      !run ||
      gateway ||
      !canSend ||
      (!handoff && (!text.trim() || textTooLarge))
    )
      return;
    const next = await command("cli_run_send", {
      request: {
        requestId: crypto.randomUUID(),
        runId: run.id,
        expectedRevision: run.revision,
        text: handoff ? "" : text,
        handoff,
      },
    });
    if (next && !handoff) setText("");
  }
  return (
    <>
      {removing && (
        <Modal
          title="Remove saved run?"
          tone="warning"
          protectTheme
          initialFocus={cancelRemoval}
          closeDisabled={busy}
          onClose={() => setRemoving(undefined)}
        >
          <div className="dialog-form">
            <p>
              Remove “{removing.title}” and its saved messages, output and
              account attempts from history? CLI files stay on disk.
            </p>
            {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
            <div className="dialog-actions">
              <button
                ref={cancelRemoval}
                className="button"
                disabled={busy}
                onClick={() => setRemoving(undefined)}
              >
                Cancel
              </button>
              <button
                className="button button-danger"
                disabled={busy}
                onClick={async () => {
                  if (
                    await command("cli_run_remove", {
                      request: {
                        requestId: crypto.randomUUID(),
                        runId: removing.id,
                        expectedRevision: removing.revision,
                      },
                    })
                  ) {
                    setRemoving(undefined);
                    setSelectedRun("");
                  }
                }}
              >
                Remove from history
              </button>
            </div>
          </div>
        </Modal>
      )}
      <Modal
        title={historyOpen ? "Run history" : "New run"}
        className="router-dialog router-launch-dialog"
        wide
        onClose={onClose}
        closeDisabled={busy}
      >
        <div className="dialog-form router-run-form" aria-busy={busy}>
          <div className="router-context" title={cwd}>
            <Folder size={14} aria-hidden="true" />
            <span>{cwd.split(/[\\/]/).filter(Boolean).at(-1) || cwd}</span>
          </div>
          <div
            className="router-view-switch"
            role="group"
            aria-label="Router views"
          >
            <button
              type="button"
              className="button"
              aria-pressed={!historyOpen}
              disabled={busy}
              onClick={() => {
                setHistoryOpen(false);
                setSelectedRun("");
                setStep(0);
              }}
            >
              <Plus size={14} aria-hidden="true" /> New run
            </button>
            <button
              type="button"
              className="button"
              aria-pressed={historyOpen}
              disabled={busy}
              onClick={() => {
                setHistoryOpen(true);
                if (!selectedRun) setSelectedRun(runs[0]?.id ?? "");
              }}
            >
              <RotateCcw size={14} aria-hidden="true" /> History
              <span className="router-count">{runs.length}</span>
            </button>
          </div>
          {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
          {!snapshot && !error && <p role="status">Loading runs…</p>}
          {historyOpen && runs.length > 0 && (
            <>
              <div
                className="router-launch-history"
                role="group"
                aria-label="Saved runs"
              >
                {runs.map((item) => (
                  <button
                    type="button"
                    key={item.id}
                    className="router-launch-history-row"
                    aria-pressed={selectedRun === item.id}
                    disabled={busy}
                    onClick={() => setSelectedRun(item.id)}
                  >
                    <RotateCcw size={16} aria-hidden="true" />
                    <span>{item.title}</span>
                    <span className="router-state" data-state={item.state}>
                      {runStateLabels[item.state]}
                    </span>
                    <ChevronRight size={14} aria-hidden="true" />
                  </button>
                ))}
              </div>
              <details className="router-disclosure router-launch-history-selector">
                <summary>All runs</summary>

                <label>
                  Run history
                  <select
                    value={selectedRun}
                    onChange={(event) => setSelectedRun(event.target.value)}
                  >
                    <option value="">Choose a saved run</option>
                    {runs.map((item) => (
                      <option key={item.id} value={item.id}>
                        {item.title} · {runStateLabels[item.state]}
                      </option>
                    ))}
                  </select>
                </label>
              </details>
            </>
          )}
          {historyOpen && !run && (
            <div className="router-empty-state">
              <strong>
                {runs.length ? "Choose a saved run" : "No saved runs yet"}
              </strong>
              <p className="settings-help">
                Your runs in this folder will appear here.
              </p>
              <button
                type="button"
                className="button"
                onClick={() => {
                  setHistoryOpen(false);
                  setSelectedRun("");
                  setStep(0);
                }}
              >
                New run
              </button>
            </div>
          )}
          {run && onOpenRun && (
            <button
              className="button button-primary"
              disabled={busy}
              onClick={() => onOpenRun(run)}
            >
              Open saved run in workspace
            </button>
          )}
          {!historyOpen ? (
            <form
              className="router-run-form router-launch-flow"
              onSubmit={async (event) => {
                event.preventDefault();
                if (busy) return;
                if (step !== 2) {
                  nextStep();
                  return;
                }
                if (!routerId || !routers.some((item) => item.id === routerId))
                  return;
                if (terminalMode) {
                  if (
                    await command("cli_router_open_terminal", {
                      routerId,
                      cwd,
                      shellProfileId,
                    })
                  )
                    onClose();
                  return;
                }
                if (!validModel || !validReasoning || !validExecution) return;
                const requestId = crypto.randomUUID();
                setPendingRunId(requestId);
                const next = await command("cli_run_start", {
                  request: {
                    requestId,
                    routerId,
                    cwd,
                    shellProfileId,
                    model: model.trim(),
                    executionMode,
                    reasoningEffort: requiresReasoning ? reasoningEffort : null,
                    title: title.trim() || "CLI run",
                  },
                });
                const created = next?.runs.find(
                  (item) => item.id === requestId && item.cwd === cwd,
                );
                if (created && completedCreation.current !== created.id) {
                  completedCreation.current = created.id;
                  setPendingRunId(undefined);
                  if (onOpenRun) onOpenRun(created);
                  else {
                    setSelectedRun(created.id);
                    setHistoryOpen(true);
                  }
                }
              }}
            >
              <ol className="router-launch-progress" aria-label="Launch steps">
                {(terminalMode
                  ? ["Route", "Launch"]
                  : ["Route", "Work", "Model"]
                ).map((label, index) => {
                  const current = terminalMode && step === 2 ? 1 : step;
                  return (
                    <li
                      key={label}
                      aria-current={index === current ? "step" : undefined}
                      data-complete={index < current}
                    >
                      <span>
                        {index < current ? (
                          <Check size={12} aria-hidden="true" />
                        ) : (
                          index + 1
                        )}
                      </span>
                      {label}
                    </li>
                  );
                })}
              </ol>
              <h2
                className="router-launch-heading"
                ref={stepHeading}
                tabIndex={-1}
              >
                {step === 0
                  ? "Choose a route"
                  : step === 1
                    ? "How will you work?"
                    : terminalMode
                      ? "Ready to open"
                      : "Choose a model"}
              </h2>
              {step === 0 && (
                <div
                  className="router-launch-choices"
                  role="radiogroup"
                  aria-label="Router"
                >
                  {routers.map((router) => (
                    <label className="router-launch-choice" key={router.id}>
                      <input
                        type="radio"
                        name="launch-router"
                        value={router.id}
                        aria-label={router.label}
                        checked={routerId === router.id}
                        disabled={busy}
                        onChange={() => {
                          setRouterId(router.id);
                          setModel("");
                          setReasoningEffort("");
                        }}
                      />
                      <Terminal size={20} aria-hidden="true" />
                      <span>
                        <strong>{router.label}</strong>
                        <small>
                          {snapshot?.capabilities.find(
                            (item) => item.cli === router.cli,
                          )?.name ||
                            cliNames[router.cli as CliAgent] ||
                            router.cli}{" "}
                          · {router.orderedProfileIds.length} accounts
                        </small>
                      </span>
                      {routerId === router.id ? (
                        <Check size={16} aria-hidden="true" />
                      ) : (
                        <ChevronRight size={16} aria-hidden="true" />
                      )}
                    </label>
                  ))}
                  {!routers.length && snapshot && (
                    <SettingsNotice>
                      Create and enable a route in Agent control settings.
                    </SettingsNotice>
                  )}
                </div>
              )}
              {step === 1 && activeRouter && (
                <>
                  <div
                    className="router-launch-choices router-launch-mode-choices"
                    role="radiogroup"
                    aria-label="Execution mode"
                  >
                    {availableModes.map((mode) => {
                      const { Icon, label, hint } = launchModes[mode];
                      return (
                        <label className="router-launch-choice" key={mode}>
                          <input
                            type="radio"
                            name="launch-mode"
                            value={mode}
                            aria-label={label}
                            checked={executionMode === mode}
                            disabled={busy}
                            onChange={() => {
                              setExecutionMode(mode);
                              setModel("");
                              setReasoningEffort("");
                            }}
                          />
                          <Icon size={22} aria-hidden="true" />
                          <span>
                            <strong>{label}</strong>
                            <small>{hint}</small>
                          </span>
                          {executionMode === mode && (
                            <Check size={16} aria-hidden="true" />
                          )}
                        </label>
                      );
                    })}
                  </div>
                  {!validExecution && !!executionMode && (
                    <SettingsNotice tone="warning">
                      Choose an available work mode.
                    </SettingsNotice>
                  )}
                </>
              )}
              {step === 2 && activeRouter && (
                <>
                  <div className="router-launch-selection">
                    <Terminal size={14} aria-hidden="true" />
                    <span>{activeRouter.label}</span>
                    <span>
                      {terminalMode
                        ? "Terminal"
                        : executionMode && launchModes[executionMode].label}
                    </span>
                  </div>
                  {terminalMode ? (
                    <p className="settings-help">
                      Uses the next routed account. Existing sessions keep their
                      accounts.
                    </p>
                  ) : (
                    <>
                      <div className="router-model-fields">
                        <label>
                          Model
                          {modelChoices ? (
                            <select
                              required
                              value={model}
                              onChange={(event) => {
                                setModel(event.target.value);
                                setReasoningEffort("");
                              }}
                            >
                              <option value="">Choose a model</option>
                              {modelChoices.map((choice) => (
                                <option key={choice.id} value={choice.id}>
                                  {choice.id}
                                </option>
                              ))}
                            </select>
                          ) : (
                            <input
                              required
                              value={model}
                              onChange={(event) => {
                                setModel(event.target.value);
                                setReasoningEffort("");
                              }}
                              placeholder="Model supported by this CLI"
                              maxLength={120}
                            />
                          )}
                        </label>
                        {requiresReasoning && (
                          <label>
                            Reasoning effort
                            {modelChoices ? (
                              <select
                                required
                                disabled={!selectedModel}
                                value={reasoningEffort}
                                onChange={(event) =>
                                  setReasoningEffort(event.target.value)
                                }
                              >
                                <option value="">
                                  Choose reasoning effort
                                </option>
                                {reasoningChoices.map((effort) => (
                                  <option key={effort} value={effort}>
                                    {effort}
                                  </option>
                                ))}
                              </select>
                            ) : (
                              <input
                                required
                                value={reasoningEffort}
                                onChange={(event) =>
                                  setReasoningEffort(event.target.value)
                                }
                                placeholder="Reasoning effort supported by this model"
                                maxLength={40}
                              />
                            )}
                          </label>
                        )}
                      </div>

                      <details className="router-disclosure">
                        <summary>Name this run</summary>
                        <label>
                          Run name
                          <input
                            value={title}
                            onChange={(event) => setTitle(event.target.value)}
                            maxLength={120}
                            placeholder="CLI run"
                          />
                        </label>
                      </details>
                    </>
                  )}
                </>
              )}
              {activeRouter && step > 0 && capability?.reason && (
                <details className="router-disclosure">
                  <summary>CLI support details</summary>
                  <p className="settings-help">{capability.reason}</p>
                </details>
              )}
              {activeRouter &&
                step === 1 &&
                capability?.nativeAccountTerminal && (
                  <details className="router-disclosure">
                    <summary>Open a terminal instead</summary>
                    <button
                      type="button"
                      className="button"
                      disabled={busy || !routeReady}
                      onClick={async () => {
                        if (
                          await command("cli_router_open_terminal", {
                            routerId: activeRouter.id,
                            cwd,
                            shellProfileId,
                          })
                        )
                          onClose();
                      }}
                    >
                      Open native account terminal
                    </button>
                  </details>
                )}
              <div className="dialog-actions router-launch-actions">
                {step > 0 && (
                  <button
                    type="button"
                    className="button"
                    disabled={busy}
                    onClick={() => setStep(step === 2 && !terminalMode ? 1 : 0)}
                  >
                    <ArrowLeft size={14} aria-hidden="true" /> Back
                  </button>
                )}
                {step < 2 ? (
                  <button
                    key="next-step"
                    type="button"
                    className="button button-primary"
                    disabled={
                      busy || (step === 0 ? !routeReady : !validExecution)
                    }
                    onClick={nextStep}
                  >
                    Next <ChevronRight size={14} aria-hidden="true" />
                  </button>
                ) : (
                  <button
                    key="start-run"
                    type="submit"
                    className="button button-primary"
                    disabled={
                      busy ||
                      !routeReady ||
                      (!terminalMode &&
                        (!validModel || !validReasoning || !validExecution))
                    }
                  >
                    <Play size={14} aria-hidden="true" />
                    {terminalMode ? "Open routed terminal" : "Start run"}
                  </button>
                )}
              </div>
            </form>
          ) : run ? (
            <>
              <div className="router-run-summary" role="status">
                <strong>{run.title}</strong>
                <span className="router-state" data-state={run.state}>
                  {runStateLabels[run.state]}
                </span>
                <span className="router-selection-meta">
                  {cliName ?? "CLI"}
                </span>
              </div>
              {run.statusMessage && (
                <SettingsNotice>{run.statusMessage}</SettingsNotice>
              )}
              <div className="router-run-controls router-pane-toolbar">
                <label>
                  Account
                  <select
                    value={run.pinnedProfileId ?? ""}
                    disabled={busy || running}
                    onChange={(event) =>
                      void command("cli_run_pin", {
                        runId: run.id,
                        profileId: event.target.value || null,
                      })
                    }
                  >
                    <option value="">Automatic account order</option>
                    {run.allowedProfileIds.map((profileId) => (
                      <option key={profileId} value={profileId}>
                        {snapshot?.profiles.find(
                          (profile) => profile.id === profileId,
                        )?.label ?? "Unavailable account"}
                      </option>
                    ))}
                  </select>
                </label>
                {snapshot && (
                  <RunDataGrant
                    snapshot={snapshot}
                    run={run}
                    disabled={busy || running}
                    busy={busy}
                    error={error}
                    command={command}
                  />
                )}
                {native && snapshot && (
                  <NativeHandoff
                    key={run.id}
                    snapshot={snapshot}
                    run={run}
                    disabled={busy || running}
                  />
                )}
              </div>
              {coding && <CodingApproval runId={run.id} />}
              {native && <NativeApproval runId={run.id} />}
              {canAcknowledge && (
                <button
                  className="button"
                  disabled={busy}
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
              {run.activeProfileId && (
                <p className="settings-help">
                  Current account:{" "}
                  {snapshot?.profiles.find(
                    (profile) => profile.id === run.activeProfileId,
                  )?.label ?? "Unavailable account"}
                </p>
              )}
              {!!run.inputs.length && (
                <details>
                  <summary>Sent messages ({run.inputs.length})</summary>
                  <ol className="router-run-history">
                    {run.inputs.map((input) => (
                      <li key={input.id}>{input.text}</li>
                    ))}
                  </ol>
                </details>
              )}
              {!!run.attempts.length && (
                <details>
                  <summary>Account attempts ({run.attempts.length})</summary>
                  <ol className="router-run-history">
                    {run.attempts.map((attempt) => (
                      <li key={attempt.id}>
                        {snapshot?.profiles.find(
                          (profile) => profile.id === attempt.profileId,
                        )?.label ?? "Unavailable account"}{" "}
                        · {attempt.state.replaceAll("_", " ")}
                        {attempt.reason ? ` · ${attempt.reason}` : ""}
                      </li>
                    ))}
                  </ol>
                </details>
              )}
              {gateway && (
                <SettingsNotice>
                  Open in your workspace to start the CLI. Approved API accounts
                  and CLI permissions apply.
                </SettingsNotice>
              )}
              {run.output && (
                <pre className="router-run-output" aria-label="CLI output">
                  {run.output}
                </pre>
              )}
              {!gateway && recovery && (
                <SettingsNotice
                  tone="warning"
                  action={
                    <button
                      className="button"
                      disabled={busy || !canSend}
                      onClick={() => void send(true)}
                    >
                      {native
                        ? "Resume checkpointed task"
                        : "Continue saved task"}
                    </button>
                  }
                >
                  {native
                    ? "Review uncertain file changes in the original account before resuming."
                    : coding
                      ? "Recover uncertain changes before continuing with saved history and tool results."
                      : "Review partial output before a new attempt with approved saved context."}
                </SettingsNotice>
              )}
              {!gateway && native && recovery && (
                <button
                  className="button"
                  disabled={busy}
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
              {!gateway && (
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
                      maxLength={textLimit}
                      value={text}
                      placeholder="What would you like to do?"
                      onChange={(event) => setText(event.target.value)}
                      disabled={busy || running || recovery || !canSend}
                    />
                  </label>
                  {textTooLarge && (
                    <p role="alert">
                      Message exceeds the {coding ? "60" : "64"} KiB UTF-8
                      limit.
                    </p>
                  )}
                  <div className="dialog-actions">
                    <button
                      type="button"
                      className="button"
                      disabled={
                        busy ||
                        (!running &&
                          run.state !== "waiting_for_capacity" &&
                          run.state !== "recovery_required")
                      }
                      onClick={() =>
                        void command("cli_run_stop", { runId: run.id })
                      }
                    >
                      Stop
                    </button>
                    <button
                      className="button button-primary"
                      disabled={
                        busy ||
                        running ||
                        recovery ||
                        !text.trim() ||
                        textTooLarge ||
                        !canSend
                      }
                    >
                      Send
                    </button>
                  </div>
                </form>
              )}
              {gateway && running && (
                <button
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void command("cli_run_stop", { runId: run.id })
                  }
                >
                  Stop CLI
                </button>
              )}
              <div className="router-inline-actions">
                <button
                  type="button"
                  className="button"
                  disabled={busy || running}
                  onClick={() =>
                    setRemoving({
                      id: run.id,
                      title: run.title,
                      revision: run.revision,
                    })
                  }
                >
                  Remove from history
                </button>
              </div>
            </>
          ) : null}
        </div>
      </Modal>
    </>
  );
}
export { RouterRunDialog };
