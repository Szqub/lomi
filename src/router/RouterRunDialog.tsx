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
import "./router.css";
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
        title="Routed CLI runs"
        wide
        onClose={onClose}
        closeDisabled={busy}
      >
        <div className="dialog-form router-run-form" aria-busy={busy}>
          <p className="settings-help">
            Working folder: {cwd}. Closing this dialog keeps active work
            running.
          </p>
          {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
          {!snapshot && !error && <p role="status">Loading runs…</p>}
          <label>
            Run history
            <select
              value={selectedRun}
              onChange={(event) => setSelectedRun(event.target.value)}
            >
              <option value="">New run</option>
              {runs.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.title} · {item.state.replaceAll("_", " ")}
                </option>
              ))}
            </select>
          </label>
          {run && onOpenRun && (
            <button
              className="button button-primary"
              disabled={busy}
              onClick={() => onOpenRun(run)}
            >
              Open saved run in workspace
            </button>
          )}
          {!run ? (
            <form
              className="router-run-form"
              onSubmit={async (event) => {
                event.preventDefault();
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
                if (created) {
                  if (onOpenRun) onOpenRun(created);
                  else setSelectedRun(created.id);
                }
              }}
            >
              <label>
                Router
                <select
                  required
                  value={routerId}
                  onChange={(event) => setRouterId(event.target.value)}
                >
                  <option value="">Choose a router</option>
                  {routers.map((router) => (
                    <option key={router.id} value={router.id}>
                      {router.label}
                    </option>
                  ))}
                </select>
              </label>
              {!routers.length && snapshot && (
                <SettingsNotice>
                  Create and enable a supported router in Settings → Agent
                  control → Router.
                </SettingsNotice>
              )}
              {activeRouter && (
                <>
                  <SettingsNotice>
                    {terminalMode
                      ? "The router chooses an account for a new CLI terminal. Existing terminals keep their accounts; running CLI sessions never switch accounts."
                      : gateway
                        ? "Gateway runs the native CLI in a terminal and forwards its native context to approved API accounts. The CLI uses its own tools and permission prompts; Lomi’s file approval broker does not apply. Open the run in the workspace, then explicitly choose Start CLI."
                        : native
                          ? "Native coding uses CLI-managed login, built-in tools and native conversation state. Permission requests appear in Lomi; native file and shell operations follow the CLI’s own policies. Unknown tool outcomes stop automatic continuation."
                          : executionMode === "coding"
                            ? "Coding retains native conversation history. Four bounded project tools can list, read, write and apply structured patches. Every write or patch requires your explicit approval. Native shell execution is unavailable."
                            : "Text-only CLI run. Project files and tools are unavailable. Every turn starts a fresh conversation."}
                  </SettingsNotice>
                  <p className="settings-help" role="status">
                    {cliName}
                    {capability?.reason ? ` · ${capability.reason}` : ""}
                  </p>
                  {capability?.nativeAccountTerminal && (
                    <button
                      type="button"
                      className="button"
                      disabled={busy}
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
                  )}
                </>
              )}
              {!terminalMode && (
                <>
                  <label>
                    Execution mode
                    <select
                      value={executionMode}
                      onChange={(event) =>
                        setExecutionMode(
                          event.target.value as
                            "" | "text" | "coding" | "gateway" | "native",
                        )
                      }
                    >
                      <option value="">Choose an execution mode</option>
                      {capability?.managedTurns && (
                        <option value="text">Text</option>
                      )}
                      {codingAvailable && (
                        <option value="coding">Coding</option>
                      )}
                      {capability?.gatewayTerminal && (
                        <option value="gateway">Gateway CLI terminal</option>
                      )}
                      {capability?.nativeTurns && (
                        <option value="native">
                          Native subscription coding
                        </option>
                      )}
                    </select>
                  </label>
                  {!validExecution && (
                    <SettingsNotice tone="warning">
                      {executionMode === "coding"
                        ? "Coding is no longer available for this router. Select Text or another supported router."
                        : "Choose an available execution mode for this router."}
                    </SettingsNotice>
                  )}
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
                  <label>
                    Run name
                    <input
                      value={title}
                      onChange={(event) => setTitle(event.target.value)}
                      maxLength={120}
                    />
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
                          <option value="">Choose reasoning effort</option>
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
                </>
              )}
              {!terminalMode && activeRouter && (
                <p className="settings-help">
                  {requiresReasoning
                    ? "Choose a model and its reasoning effort to start."
                    : "Choose a model to start."}
                </p>
              )}
              <div className="dialog-actions">
                <button
                  className="button button-primary"
                  disabled={
                    busy ||
                    !routerId ||
                    (!terminalMode &&
                      (!validModel || !validReasoning || !validExecution))
                  }
                >
                  {terminalMode ? "Open routed terminal" : "Start run"}
                </button>
              </div>
            </form>
          ) : (
            <>
              <p role="status">
                {run.title} · {cliName ?? "CLI"} ·{" "}
                {run.state.replaceAll("_", " ")}
                {run.statusMessage ? ` · ${run.statusMessage}` : ""}
              </p>
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
              {coding && (
                <>
                  <p className="settings-help">
                    Native history is retained. Four bounded project tools list,
                    read, write and apply structured patches; every write or
                    patch needs explicit approval. Native shell execution is
                    unavailable.
                  </p>
                  <CodingApproval runId={run.id} />
                </>
              )}
              {native && snapshot && (
                <NativeHandoff
                  key={run.id}
                  snapshot={snapshot}
                  run={run}
                  disabled={busy || running}
                />
              )}
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
                  Gateway uses the native CLI’s tools and permission prompts and
                  sends native context to approved APIs. Open this saved run in
                  the workspace, then choose Start CLI. Restored views never
                  start a terminal automatically. A previously launched session
                  retains private native history and requires a new run for
                  another launch.
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
                    ? "Resume requires a settled checkpoint in the original account. Review uncertain file changes and tool results in the exact saved native session; the original task is not replayed automatically."
                    : coding
                      ? "Continue retains native history and recorded tool results without resending the original task. Unknown dispatches or effects remain blocked until recovered. Acknowledge recorded completion returns a fully completed input to idle without launching a turn."
                      : "The saved task may have produced partial output. Continuing starts a fresh CLI attempt using saved context and the selected account policy; review completed work first."}
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
          )}
        </div>
      </Modal>
    </>
  );
}
export { RouterRunDialog };
