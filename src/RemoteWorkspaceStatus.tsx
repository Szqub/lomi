import { useId, useRef, useState } from "react";
import { Globe } from "./icons";
import { Modal } from "./ui";

export default function RemoteWorkspaceStatus({
  workspace,
  available,
  connectionMessage,
  busy,
  error,
  stopSharing,
  paused,
  resume,
}: {
  workspace: { id: string; name: string };
  available: boolean;
  connectionMessage: string;
  busy: boolean;
  error: string;
  stopSharing: () => Promise<void>;
  paused?: boolean;
  resume: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const cancel = useRef<HTMLButtonElement>(null);
  const descriptionId = useId();
  return (
    <>
      <div
        className="remote-workspace-status"
        title={`${workspace.name}: ${connectionMessage}`}
      >
        <span className="remote-workspace-status-label">
          <Globe size={12} aria-hidden="true" />
          {paused ? "Remote paused" : "Shared remotely"}
        </span>
        {paused && (
          <button
            type="button"
            className="remote-workspace-stop"
            disabled={!available || busy}
            onClick={() => void resume()}
          >
            {busy ? "Resuming…" : "Resume remote"}
          </button>
        )}
        <button
          type="button"
          className="remote-workspace-stop"
          aria-label="Stop sharing remotely"
          aria-haspopup="dialog"
          disabled={!available || busy}
          onClick={(event) => {
            event.currentTarget.focus();
            setAttempted(false);
            setOpen(true);
          }}
        >
          Stop sharing
        </button>
      </div>
      {paused && error && <span role="alert">{error}</span>}
      {open && (
        <Modal
          className="remote-sharing-dialog"
          title="Stop sharing remotely?"
          role="alertdialog"
          tone="warning"
          protectTheme
          descriptionId={descriptionId}
          initialFocus={cancel}
          closeDisabled={busy}
          onClose={() => setOpen(false)}
        >
          <div className="dialog-form" aria-busy={busy}>
            <p id={descriptionId}>
              Stop sharing “{workspace.name}” remotely? Remote access will end.
              Local terminals will keep running.
              {!available && " Sign in to use remote sharing."}
            </p>
            {attempted && error && <p role="alert">{error}</p>}
            <div className="dialog-actions">
              <button
                ref={cancel}
                type="button"
                className="button"
                disabled={busy}
                onClick={() => setOpen(false)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="button button-primary button-danger"
                disabled={!available || busy}
                onClick={() => {
                  if (!available) return;
                  setAttempted(true);
                  void stopSharing();
                }}
              >
                {busy ? "Stopping…" : "Stop sharing"}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </>
  );
}
