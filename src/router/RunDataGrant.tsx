import { useId, useRef, useState } from "react";
import { Modal } from "../ui";
import { SettingsNotice } from "../settings-ui";
import type { CliProfile, CliRouterSnapshot, CliRun } from "./types";
import { compatibleGatewayProtocol } from "./model";

interface ReviewedAccount {
  id: string;
  label: string;
  revision?: number;
  approved: boolean;
  selectable: boolean;
}
interface Review {
  runId: string;
  revision: number;
  accounts: ReviewedAccount[];
}

function eligible(
  profile: CliProfile,
  cli: string,
  managed: boolean,
  gatewayProtocol?: string | null,
  native = false,
) {
  return (
    managed &&
    profile.cli === cli &&
    profile.enabled &&
    (profile.authState === "ready" ||
      (native && profile.authState === "unverified")) &&
    (native
      ? profile.storageMode !== "api_key" && !profile.gatewayProvider
      : gatewayProtocol
        ? profile.storageMode === "api_key" &&
          compatibleGatewayProtocol(
            gatewayProtocol,
            profile.gatewayProvider?.protocol,
            cli,
          )
        : !profile.gatewayProvider &&
          (cli === "codex"
            ? profile.storageMode !== "api_key"
            : profile.storageMode === "api_key"))
  );
}

export default function RunDataGrant({
  snapshot,
  run,
  disabled,
  busy,
  error,
  command,
}: {
  snapshot: CliRouterSnapshot;
  run: CliRun;
  disabled: boolean;
  busy: boolean;
  error?: string;
  command: (
    name: string,
    args: Record<string, unknown>,
  ) => Promise<CliRouterSnapshot | undefined>;
}) {
  const [review, setReview] = useState<Review>();
  const [selected, setSelected] = useState<string[]>([]);
  const cancel = useRef<HTMLButtonElement>(null);
  const description = useId();
  const stale =
    !!review &&
    (run.id !== review.runId ||
      run.revision !== review.revision ||
      review.accounts.some(
        (account) =>
          account.revision !==
          snapshot.profiles.find((profile) => profile.id === account.id)
            ?.revision,
      ));
  function openReview() {
    const router = snapshot.routers.find((item) => item.id === run.routerId);
    const capability = snapshot.capabilities.find(
      (item) => item.cli === router?.cli,
    );
    const gatewayProtocol =
      run.executionMode === "gateway" && capability?.gatewayTerminal
        ? capability.gatewayProtocol
        : null;
    const native = run.executionMode === "native" && !!capability?.nativeTurns;
    const managed = !!(native || gatewayProtocol || capability?.managedTurns);
    const ids = [
      ...new Set([
        ...run.allowedProfileIds,
        ...(router?.orderedProfileIds ?? []),
      ]),
    ];
    setReview({
      runId: run.id,
      revision: run.revision,
      accounts: ids.map((id) => {
        const profile = snapshot.profiles.find((item) => item.id === id);
        const approved = run.allowedProfileIds.includes(id);
        return {
          id,
          label: profile?.label ?? "Unavailable account",
          revision: profile?.revision,
          approved,
          selectable:
            approved ||
            (!!router?.enabled &&
              !!profile &&
              eligible(profile, router.cli, managed, gatewayProtocol, native)),
        };
      }),
    });
    setSelected([...run.allowedProfileIds]);
  }
  return (
    <>
      <button
        type="button"
        className="button"
        disabled={disabled}
        onClick={openReview}
      >
        Review account access
      </button>
      {review && (
        <Modal
          title="Review access to saved history"
          tone="warning"
          protectTheme
          initialFocus={cancel}
          descriptionId={description}
          closeDisabled={busy}
          onClose={() => setReview(undefined)}
        >
          <form
            className="dialog-form"
            aria-busy={busy}
            onSubmit={async (event) => {
              event.preventDefault();
              if (disabled || stale) return;
              const next = await command("cli_run_update_data_grant", {
                request: {
                  requestId: crypto.randomUUID(),
                  runId: review.runId,
                  expectedRevision: review.revision,
                  allowedProfileIds: selected,
                  profiles: review.accounts
                    .filter(
                      (account) =>
                        selected.includes(account.id) &&
                        account.revision !== undefined,
                    )
                    .map((account) => ({
                      profileId: account.id,
                      expectedRevision: account.revision,
                    })),
                },
              });
              if (next) setReview(undefined);
            }}
          >
            <p id={description}>
              {run.executionMode === "gateway"
                ? "Selected accounts may receive the complete native CLI context when you explicitly start this fresh terminal session. The CLI controls its tools and permission prompts."
                : "Selected accounts may receive all saved messages and responses, including uncertain partial responses, when you next Send or Continue this task."}{" "}
              Saving access does not start an attempt.
            </p>
            <fieldset
              className="router-grant-accounts"
              disabled={disabled || stale}
            >
              <legend>Accounts allowed to receive this run’s history</legend>
              {review.accounts.map((account) => (
                <label key={account.id}>
                  <input
                    type="checkbox"
                    checked={selected.includes(account.id)}
                    disabled={!account.selectable}
                    onChange={(event) =>
                      setSelected((current) =>
                        event.target.checked
                          ? [...current, account.id]
                          : current.filter((id) => id !== account.id),
                      )
                    }
                  />
                  <span>
                    {account.label}
                    {account.approved
                      ? " · already approved"
                      : account.selectable
                        ? " · new access"
                        : " · unavailable for this task"}
                  </span>
                </label>
              ))}
            </fieldset>
            {stale && (
              <SettingsNotice tone="warning">
                The task or an account changed. Cancel and review the current
                history and accounts again.
              </SettingsNotice>
            )}
            {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
            <div className="dialog-actions">
              <button
                ref={cancel}
                type="button"
                className="button"
                disabled={busy}
                onClick={() => setReview(undefined)}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={disabled || stale}
              >
                Save account access
              </button>
            </div>
          </form>
        </Modal>
      )}
    </>
  );
}
