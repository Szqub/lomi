import { useEffect, useState } from "react";
import { api, errorMessage, native } from "./api";
import type { RemoteState } from "./remote-workspace-domain";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

export default function RemoteSettingsPage() {
  const [state, setState] = useState<RemoteState | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!native) return;
    let current = true;
    const refresh = () =>
      void api<RemoteState>("remote_get_state")
        .then((next) => {
          if (current) setState(next);
        })
        .catch((e) => {
          if (current) setError(errorMessage(e));
        });
    refresh();
    const timer = setInterval(refresh, 2000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, []);
  const revoke = async (grantId: string) => {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      setState(await api<RemoteState>("remote_revoke_grant", { grantId }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <SettingsPage
      title="Remote"
      status={state?.online ? "Online" : state?.enabled ? "Connecting" : "Off"}
    >
      {error && (
        <SettingsNotice tone="error" role="alert">
          {error}
        </SettingsNotice>
      )}
      {!native ? (
        <SettingsNotice>
          Remote hosting is available in the desktop app.
        </SettingsNotice>
      ) : (
        <>
          <SettingsSection title="Workspace access">
            <SettingsNotice>
              Right-click a workspace and choose Share remotely. Browsers signed
              in to the same account can view and control all its terminals,
              including inactive tabs and new splits. Choose Stop sharing
              remotely to end access.
            </SettingsNotice>
            <SettingsNotice>
              Terminal contents and input are encrypted between your devices.
              Local typing immediately takes control back.
            </SettingsNotice>
            {state?.message && <SettingsNotice>{state.message}</SettingsNotice>}
            <SettingRow
              label="Keep Lomi running"
              description="Closing the workspace hides its window and keeps its terminals running. Quit Lomi saves and stops sessions through the existing close guards."
            >
              <button
                className="button"
                onClick={() =>
                  void api("reopen_main_window").catch((e) =>
                    setError(errorMessage(e)),
                  )
                }
              >
                Show workspace
              </button>
              <button
                className="button"
                onClick={() =>
                  void api("request_quit").catch((e) =>
                    setError(errorMessage(e)),
                  )
                }
              >
                Quit Lomi
              </button>
            </SettingRow>
          </SettingsSection>
          <SettingsSection title="Shared workspaces">
            {!state?.workspaces?.some((w) => w.shared) && (
              <SettingsNotice>No workspaces are shared.</SettingsNotice>
            )}
            {state?.workspaces
              ?.filter((w) => w.shared)
              .map((workspace, index) => (
                <SettingRow
                  key={workspace.id}
                  label={`Workspace ${index + 1}`}
                  description={
                    workspace.message ??
                    (workspace.online
                      ? "Available to your account"
                      : "Waiting for connection")
                  }
                >
                  <span>{workspace.online ? "Online" : "Offline"}</span>
                </SettingRow>
              ))}
          </SettingsSection>
          <SettingsSection title="Connected browsers">
            {!state?.grants?.some((grant) => !grant.revoked) && (
              <SettingsNotice>No browsers have access.</SettingsNotice>
            )}
            {state?.grants
              ?.filter((grant) => !grant.revoked)
              .map((grant, index) => (
                <SettingRow
                  key={grant.id}
                  label={`Browser ${index + 1}`}
                  description={`${grant.sessionIds.length} terminals · expires ${new Date(grant.expiresAt * 1000).toLocaleString()}`}
                >
                  <button
                    className="button"
                    disabled={busy}
                    onClick={() => void revoke(grant.id)}
                  >
                    Revoke access
                  </button>
                </SettingRow>
              ))}
          </SettingsSection>
        </>
      )}
    </SettingsPage>
  );
}
