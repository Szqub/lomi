import {
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { errorMessage } from "../api";
import { Modal } from "../ui";
import { SettingsNotice } from "../settings-ui";
import { getRunView } from "./run-runtime";
import type { CliRouterSnapshot, CliRun } from "./types";

interface HandoffPreview {
  runId: string;
  runRevision: number;
  sourceProfileId: string;
  sourceProfileRevision: number;
  sourceLabel: string;
  destinationProfileId: string;
  destinationProfileRevision: number;
  destinationLabel: string;
  generation: number;
  inputId: string;
  model: string;
  version: string;
  historyDigest: string;
  historyBytes: number;
}

export default function NativeHandoff({
  snapshot,
  run,
  disabled,
}: {
  snapshot: CliRouterSnapshot;
  run: CliRun;
  disabled: boolean;
}) {
  const runtime = getRunView(run.id);
  const state = useSyncExternalStore(runtime.subscribe, runtime.getSnapshot);
  const [open, setOpen] = useState(false);
  const [destinationId, setDestinationId] = useState("");
  const [preview, setPreview] = useState<HandoffPreview>();
  const [consent, setConsent] = useState(false);
  const [loading, setLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState("");
  const [applyError, setApplyError] = useState("");
  const [transferred, setTransferred] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  const sequence = useRef(0);
  const description = useId();
  const destinationHelp = useId();
  const router = snapshot.routers.find((item) => item.id === run.routerId);
  const sourceId = run.activeProfileId ?? run.attempts.at(-1)?.profileId;
  const source = snapshot.profiles.find((item) => item.id === sourceId);
  const eligible =
    run.executionMode === "native" &&
    router?.cli === "pi" &&
    router.enabled &&
    (run.state === "idle" ||
      run.state === "completed" ||
      run.state === "recovery_required") &&
    !!sourceId;
  const destinations = snapshot.profiles.filter(
    (profile) =>
      profile.id !== sourceId &&
      profile.cli === "pi" &&
      profile.enabled &&
      profile.storageMode !== "api_key" &&
      !profile.gatewayProvider &&
      (profile.authState === "ready" || profile.authState === "unverified") &&
      router?.orderedProfileIds.includes(profile.id) &&
      run.allowedProfileIds.includes(profile.id),
  );
  const destination = destinations.find((item) => item.id === destinationId);
  const busy = disabled || state.busy || applying;
  const context = JSON.stringify([
    run.id,
    run.revision,
    run.generation,
    run.model,
    run.state,
    eligible,
    sourceId,
    source?.revision,
    router?.revision,
    destinations.map((item) => [item.id, item.revision]),
    destinationId,
  ]);
  const currentContext = useRef(context);
  currentContext.current = context;
  const previewValid =
    !!preview &&
    eligible &&
    !!destination &&
    preview.runId === run.id &&
    preview.runRevision === run.revision &&
    preview.generation === run.generation &&
    preview.model === run.model &&
    preview.inputId === run.inputs.at(-1)?.id &&
    preview.sourceProfileId === sourceId &&
    preview.sourceProfileRevision === source?.revision &&
    preview.destinationProfileId === destination.id &&
    preview.destinationProfileRevision === destination.revision;

  useEffect(() => {
    sequence.current += 1;
    setPreview(undefined);
    setConsent(false);
    setLoading(false);
    setError("");
  }, [context]);
  useEffect(() => {
    setOpen(false);
    setDestinationId("");
    setTransferred("");
    setApplyError("");
  }, [run.id]);
  useEffect(
    () => () => {
      sequence.current += 1;
    },
    [],
  );

  function close() {
    if (applying) return;
    sequence.current += 1;
    setOpen(false);
    setPreview(undefined);
    setConsent(false);
    setLoading(false);
    setError("");
    setApplyError("");
  }
  async function review() {
    if (busy || loading || !eligible || !destination) return;
    const requestSequence = ++sequence.current;
    const requestContext = context;
    setPreview(undefined);
    setConsent(false);
    setLoading(true);
    setError("");
    setApplyError("");
    try {
      const next = await invoke<HandoffPreview>("cli_native_handoff_preview", {
        request: {
          runId: run.id,
          expectedRevision: run.revision,
          destinationProfileId: destination.id,
          destinationProfileRevision: destination.revision,
        },
      });
      if (
        requestSequence === sequence.current &&
        currentContext.current === requestContext
      )
        setPreview(next);
    } catch (cause) {
      if (
        requestSequence === sequence.current &&
        currentContext.current === requestContext
      )
        setError(errorMessage(cause));
    } finally {
      if (requestSequence === sequence.current) setLoading(false);
    }
  }
  async function apply() {
    if (busy || loading || !previewValid || !consent || !preview) return;
    const reviewed = preview;
    setApplying(true);
    setError("");
    setApplyError("");
    try {
      const next = await runtime.command("cli_native_handoff_apply", {
        request: { requestId: crypto.randomUUID(), review: reviewed },
      });
      if (next) {
        setTransferred(reviewed.destinationLabel);
        setOpen(false);
        setPreview(undefined);
        setConsent(false);
      } else {
        setApplyError(
          runtime.getSnapshot().error ||
            "The transfer was not saved. Review the current history again.",
        );
        setPreview(undefined);
        setConsent(false);
      }
    } finally {
      setApplying(false);
    }
  }
  if (!eligible && !open && !transferred) return null;
  return (
    <>
      {eligible && destinations.length > 0 && (
        <button
          type="button"
          className="button"
          disabled={busy}
          onClick={() => {
            setDestinationId("");
            setPreview(undefined);
            setConsent(false);
            setError("");
            setApplyError("");
            setTransferred("");
            setOpen(true);
          }}
        >
          Transfer Pi native history
        </button>
      )}
      {transferred && (
        <p role="status" className="settings-help">
          Native history transferred to {transferred}. Send a message or choose
          Resume checkpointed task to start a turn.
        </p>
      )}
      {open && (
        <Modal
          title="Transfer Pi native history"
          tone="warning"
          protectTheme
          descriptionId={description}
          initialFocus={cancel}
          closeDisabled={applying}
          onClose={close}
        >
          <form
            className="dialog-form"
            aria-busy={loading || applying}
            onSubmit={(event) => {
              event.preventDefault();
              void apply();
            }}
          >
            <p id={description}>
              Copy this run’s saved native conversation and tool results to an
              approved Pi account. Pi retains its native tools and permission
              prompts. Only compatible OpenAI-backed Pi sessions support this
              transfer.
            </p>
            <label>
              Destination account
              <select
                required
                value={destinationId}
                disabled={busy || !eligible}
                aria-describedby={destinationHelp}
                onChange={(event) => {
                  sequence.current += 1;
                  setDestinationId(event.target.value);
                  setPreview(undefined);
                  setConsent(false);
                  setLoading(false);
                  setError("");
                  setApplyError("");
                }}
              >
                <option value="">Choose an approved account</option>
                {destinations.map((profile) => (
                  <option key={profile.id} value={profile.id}>
                    {profile.label}
                  </option>
                ))}
              </select>
            </label>
            <p id={destinationHelp} className="settings-help">
              Only other eligible accounts already allowed to receive this run’s
              history are listed. Use Review account access to grant new access.
            </p>
            {!eligible && (
              <SettingsNotice tone="warning">
                This run changed. Transfer requires a settled native Pi run.
              </SettingsNotice>
            )}
            <button
              type="button"
              className="button"
              disabled={busy || loading || !eligible || !destination}
              onClick={() => void review()}
            >
              {loading ? "Reviewing saved history…" : "Review transfer"}
            </button>
            {loading && (
              <p role="status">Reading the saved native checkpoint…</p>
            )}
            {preview && !previewValid && (
              <SettingsNotice tone="warning">
                The run or an account changed. Review the current transfer
                again.
              </SettingsNotice>
            )}
            {previewValid && preview && (
              <>
                <p>
                  From <strong>{preview.sourceLabel}</strong> to{" "}
                  <strong>{preview.destinationLabel}</strong> · {preview.model}{" "}
                  · Pi {preview.version} ·{" "}
                  {preview.historyBytes.toLocaleString()} bytes of saved native
                  history.
                </p>
                <fieldset className="router-grant-accounts" disabled={busy}>
                  <legend>History and account consent</legend>
                  <label>
                    <input
                      type="checkbox"
                      checked={consent}
                      disabled={busy}
                      onChange={(event) => setConsent(event.target.checked)}
                    />
                    <span>
                      I approve copying this native history from{" "}
                      {preview.sourceLabel} to {preview.destinationLabel} and
                      using that account for later turns.
                    </span>
                  </label>
                </fieldset>
              </>
            )}
            <p className="settings-help">
              Transfer saves history and account selection. Start the next turn
              separately with Send or Resume checkpointed task.
            </p>
            {(error || applyError) && (
              <div role="alert">
                <SettingsNotice tone="error">
                  {error || applyError}
                </SettingsNotice>
              </div>
            )}
            <div className="dialog-actions">
              <button
                ref={cancel}
                type="button"
                className="button"
                disabled={applying}
                onClick={close}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={busy || loading || !previewValid || !consent}
              >
                {applying ? "Transferring history…" : "Transfer native history"}
              </button>
            </div>
          </form>
        </Modal>
      )}
    </>
  );
}
