import { cliNames, type CliAgent } from "../cli-agents";
import { useEffect, useState } from "react";
import { SettingRow, SettingsNotice, SettingsSection } from "../settings-ui";
import {
  moveAccount,
  quotaLabel,
  isQuotaFresh,
  hasComparableQuota,
  gatewayProtocolsForCli,
} from "./model";
import type { GatewayProtocol } from "./types";
import { invoke } from "@tauri-apps/api/core";
import { useRouterSnapshot } from "./useRouterSnapshot";
import "./router.css";
interface NativeReport {
  profileId: string;
  profileRevision: number;
  observedAt: number;
  expiresAt: number;
  authenticated: boolean;
  observation: {
    source: string;
    identity?: { displayLabel: string } | null;
    windows: {
      id: string;
      name: string;
      nativeId?: string | null;
      nativeWindow?: string | null;
      remainingPercent?: number | null;
      remainingAmount?: number | null;
      unit?: string | null;
      disabled: boolean;
      resetAt?: number | null;
    }[];
  };
  error?: string;
}
export default function RouterSettings() {
  const { snapshot, error, busy, mutate, command } = useRouterSnapshot();
  const [now, setNow] = useState(Date.now);
  useEffect(() => setNow(Date.now()), [snapshot]);
  const hasFreshReports =
    snapshot?.quota.some((quota) => isQuotaFresh(quota, now)) ?? false;
  useEffect(() => {
    if (!hasFreshReports) return;
    const clock = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(clock);
  }, [hasFreshReports]);
  const [search, setSearch] = useState("");
  const [cli, setCli] = useState("");
  const [accountLabel, setAccountLabel] = useState("");
  const [poolLabel, setPoolLabel] = useState("");
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [endpoints, setEndpoints] = useState<Record<string, string>>({});
  const [protocols, setProtocols] = useState<Record<string, GatewayProtocol>>(
    {},
  );
  const [nativeReports, setNativeReports] = useState<
    Record<string, NativeReport>
  >({});
  const [nativeBusy, setNativeBusy] = useState<string>();
  const [nativeError, setNativeError] = useState("");
  const capabilities = snapshot?.capabilities ?? [];
  const selected = capabilities.find((item) => item.cli === cli);
  const profiles = snapshot?.profiles.filter((item) => item.cli === cli) ?? [];
  const routers = snapshot?.routers.filter((item) => item.cli === cli) ?? [];
  const nativeBindings = profiles
    .filter((profile) => profile.storageMode === "cli_managed")
    .map((profile) => `${profile.id}:${profile.revision}`)
    .join(",");
  useEffect(() => {
    let live = true;
    const ids = nativeBindings
      ? nativeBindings.split(",").map((binding) => binding.split(":")[0])
      : [];
    void Promise.all(
      ids.map(async (profileId) => {
        try {
          return await invoke<NativeReport | null>(
            "cli_profile_native_report",
            { profileId },
          );
        } catch {
          return null;
        }
      }),
    ).then((reports) => {
      if (!live) return;
      setNativeReports(
        Object.fromEntries(
          reports
            .filter((report): report is NativeReport => !!report)
            .map((report) => [report.profileId, report]),
        ),
      );
    });
    return () => {
      live = false;
    };
  }, [nativeBindings]);
  async function refreshNative(profileId: string) {
    if (nativeBusy) return;
    setNativeBusy(profileId);
    setNativeError("");
    try {
      const report = await invoke<NativeReport>("cli_profile_refresh_native", {
        profileId,
      });
      setNativeReports((current) => ({ ...current, [profileId]: report }));
      await command("cli_router_snapshot", {});
    } catch (failure) {
      setNativeError(String(failure));
    } finally {
      setNativeBusy(undefined);
    }
  }
  return (
    <div className="router-settings" aria-busy={busy}>
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      {nativeError && (
        <SettingsNotice tone="error">{nativeError}</SettingsNotice>
      )}
      {!snapshot && !error && <p role="status">Loading routers…</p>}
      <SettingsSection
        title="CLI routers"
        description="Keep separate CLI accounts and choose their order for routed runs."
      >
        <SettingRow label="Search CLI agents" htmlFor="router-search">
          <input
            id="router-search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </SettingRow>
        <SettingRow label="CLI agent" htmlFor="router-cli">
          <select
            id="router-cli"
            value={cli}
            onChange={(event) => setCli(event.target.value)}
          >
            <option value="">Choose a CLI agent</option>
            {capabilities
              .filter((item) =>
                `${item.name || cliNames[item.cli as CliAgent] || item.cli} ${item.cli}`
                  .toLowerCase()
                  .includes(search.toLowerCase()),
              )
              .map((item) => (
                <option key={item.cli} value={item.cli}>
                  {item.name || cliNames[item.cli as CliAgent] || item.cli}
                </option>
              ))}
          </select>
        </SettingRow>
        {!cli && (
          <p className="settings-help">
            Choose an agent to see account and routing support.
          </p>
        )}
        {selected && (
          <SettingsNotice>
            {selected.reason ||
              "Separate accounts are not supported for this CLI yet."}
          </SettingsNotice>
        )}
        {selected?.canCreateProfile && (
          <>
            <form
              className="router-create"
              onSubmit={async (event) => {
                event.preventDefault();
                if (
                  accountLabel.trim() &&
                  (await mutate({
                    type: "create_profile",
                    cli,
                    label: accountLabel.trim(),
                  }))
                )
                  setAccountLabel("");
              }}
            >
              <label htmlFor="router-account-label">Account name</label>
              <input
                id="router-account-label"
                required
                value={accountLabel}
                onChange={(event) => setAccountLabel(event.target.value)}
                maxLength={120}
              />
              <button
                className="button"
                disabled={busy || !accountLabel.trim()}
              >
                Add account
              </button>
            </form>
            <p className="settings-help">
              {selected.profileTerminal === false
                ? "Configure an API key for this CLI. Its ordinary login terminal is unavailable for router profiles. "
                : selected.canVerifyLogin
                  ? "Open the CLI and sign in using its own login flow, then verify the account. "
                  : "This CLI has no supported login check. Its isolated terminal remains available. "}
              {selected.gatewayTerminal &&
                "For Gateway CLI terminals, save the protocol endpoint first, then save its API key. Native CLI tools and permission prompts apply. "}
              {selected.managedTurns &&
                selected.apiKeyLabel &&
                "Save your own API key in the system keyring to use routed text turns. Verification checks local key presence. Every turn starts a fresh conversation."}
              {selected.cli === "codex" &&
                "Managed text turns require a verified subscription account and a current native quota report. Verify the account, then refresh quota before starting a run."}
            </p>
            <p className="settings-help">
              Removing an account deletes Lomi’s saved API key when cleanup
              completes. CLI login files remain.
              {selected.profileTerminal !== false &&
                " To sign out, use the account’s own CLI terminal first."}
            </p>
            {profiles.map((profile) => (
              <SettingRow
                key={profile.id}
                stacked
                className="router-profile-row"
                label={profile.label}
                description={`${profile.authState.replaceAll("_", " ")} · ${
                  profile.storageMode === "api_key"
                    ? "Own API key"
                    : quotaLabel(
                        snapshot?.quota.find(
                          (item) => item.profileId === profile.id,
                        ),
                        now,
                      )
                }`}
              >
                <label>
                  <input
                    type="checkbox"
                    aria-label={`Enable ${profile.label}`}
                    checked={profile.enabled}
                    disabled={busy || profile.authState === "pending_remove"}
                    onChange={(event) =>
                      void mutate({
                        type: "update_profile",
                        profileId: profile.id,
                        enabled: event.target.checked,
                      })
                    }
                  />{" "}
                  Enabled
                </label>
                <button
                  type="button"
                  className="button"
                  disabled={
                    busy ||
                    profile.authState === "pending_remove" ||
                    profile.storageMode === "api_key" ||
                    selected.profileTerminal === false
                  }
                  onClick={() =>
                    void command("cli_profile_open_terminal", {
                      profileId: profile.id,
                    })
                  }
                >
                  Open CLI to sign in
                </button>
                <button
                  type="button"
                  className="button"
                  disabled={
                    busy ||
                    profile.authState === "pending_remove" ||
                    (!selected.canVerifyLogin &&
                      profile.storageMode !== "api_key")
                  }
                  title={
                    selected.canVerifyLogin || profile.storageMode === "api_key"
                      ? undefined
                      : "This CLI does not provide a supported login check."
                  }
                  onClick={() =>
                    void command("cli_profile_verify", {
                      profileId: profile.id,
                    })
                  }
                >
                  Verify
                </button>
                {profile.storageMode === "cli_managed" &&
                  [
                    "claude",
                    "codex",
                    "grok",
                    "kimi",
                    "kilo",
                    "opencode",
                    "pi",
                    "agy",
                  ].includes(profile.cli) && (
                    <>
                      <button
                        type="button"
                        className="button"
                        disabled={
                          busy ||
                          !!nativeBusy ||
                          profile.authState === "pending_remove"
                        }
                        onClick={() => void refreshNative(profile.id)}
                      >
                        {nativeBusy === profile.id
                          ? "Reading native account…"
                          : "Refresh native account"}
                      </button>
                      {nativeReports[profile.id] && (
                        <div className="settings-help" role="status">
                          <p>
                            Recorded native account:{" "}
                            {nativeReports[profile.id].authenticated
                              ? "authenticated"
                              : "unverified"}
                            {nativeReports[profile.id].observation.identity
                              ?.displayLabel
                              ? ` · ${nativeReports[profile.id].observation.identity!.displayLabel}`
                              : ""}
                            . Observed{" "}
                            {new Date(
                              nativeReports[profile.id].observedAt,
                            ).toLocaleTimeString()}
                            .
                          </p>
                          {nativeReports[profile.id].observation.windows.map(
                            (window) => (
                              <p key={window.id}>
                                {window.name}:{" "}
                                {window.disabled
                                  ? "disabled"
                                  : window.remainingPercent != null
                                    ? `${window.remainingPercent.toFixed(1)}% remaining`
                                    : window.remainingAmount != null
                                      ? `${window.remainingAmount}${window.unit ? ` ${window.unit}` : ""} remaining`
                                      : "unknown"}
                                {window.nativeWindow
                                  ? ` · ${window.nativeWindow}`
                                  : ""}
                                {window.resetAt != null
                                  ? ` · resets ${new Date(window.resetAt).toLocaleString()}`
                                  : ""}
                              </p>
                            ),
                          )}
                          <p>
                            {nativeReports[profile.id].error ??
                              "Native limits are shown as reported. Model scope is not qualified for percentage balancing."}
                          </p>
                        </div>
                      )}
                    </>
                  )}
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() =>
                    void mutate({
                      type: "remove_profile",
                      profileId: profile.id,
                    })
                  }
                >
                  Remove
                </button>
                {selected.gatewayTerminal && selected.gatewayProtocol && (
                  <form
                    className="router-create"
                    onSubmit={async (event) => {
                      event.preventDefault();
                      if (profile.authState === "pending_remove") return;
                      const baseUrl = (
                        endpoints[profile.id] ??
                        profile.gatewayProvider?.baseUrl ??
                        ""
                      ).trim();
                      if (!baseUrl || !selected.gatewayProtocol) return;
                      if (
                        await mutate({
                          type: "configure_gateway_account",
                          profileId: profile.id,
                          provider: {
                            protocol:
                              profile.cli === "codex"
                                ? "openai_responses"
                                : (protocols[profile.id] ??
                                  profile.gatewayProvider?.protocol ??
                                  selected.gatewayProtocol),
                            baseUrl,
                          },
                        })
                      ) {
                        setKeys((current) => ({
                          ...current,
                          [profile.id]: "",
                        }));
                        setEndpoints((current) => {
                          const next = { ...current };
                          delete next[profile.id];
                          return next;
                        });
                        setProtocols((current) => {
                          const next = { ...current };
                          delete next[profile.id];
                          return next;
                        });
                      }
                    }}
                  >
                    <label htmlFor={`router-protocol-${profile.id}`}>
                      Provider API protocol
                    </label>
                    <select
                      id={`router-protocol-${profile.id}`}
                      value={
                        profile.cli === "codex"
                          ? "openai_responses"
                          : (protocols[profile.id] ??
                            profile.gatewayProvider?.protocol ??
                            selected.gatewayProtocol)
                      }
                      disabled={busy || profile.authState === "pending_remove"}
                      onChange={(event) =>
                        setProtocols((current) => ({
                          ...current,
                          [profile.id]: event.target.value as GatewayProtocol,
                        }))
                      }
                    >
                      {gatewayProtocolsForCli(profile.cli).map((item) => (
                        <option key={item.id} value={item.id}>
                          {item.label}
                        </option>
                      ))}
                    </select>
                    <label htmlFor={`router-endpoint-${profile.id}`}>
                      Gateway API endpoint
                    </label>
                    <input
                      id={`router-endpoint-${profile.id}`}
                      type="url"
                      required
                      maxLength={2048}
                      value={
                        endpoints[profile.id] ??
                        profile.gatewayProvider?.baseUrl ??
                        ""
                      }
                      disabled={busy || profile.authState === "pending_remove"}
                      onChange={(event) =>
                        setEndpoints((current) => ({
                          ...current,
                          [profile.id]: event.target.value,
                        }))
                      }
                      placeholder="https://api.example.com"
                    />
                    <button
                      className="button"
                      disabled={
                        busy ||
                        profile.authState === "pending_remove" ||
                        !(
                          endpoints[profile.id] ??
                          profile.gatewayProvider?.baseUrl ??
                          ""
                        ).trim()
                      }
                    >
                      Save gateway endpoint
                    </button>
                    {profile.gatewayProvider && (
                      <button
                        type="button"
                        className="button"
                        disabled={
                          busy || profile.authState === "pending_remove"
                        }
                        onClick={async () => {
                          if (
                            await mutate({
                              type: "configure_gateway_account",
                              profileId: profile.id,
                              provider: null,
                            })
                          ) {
                            setEndpoints((current) => ({
                              ...current,
                              [profile.id]: "",
                            }));
                            setKeys((current) => ({
                              ...current,
                              [profile.id]: "",
                            }));
                            setProtocols((current) => {
                              const next = { ...current };
                              delete next[profile.id];
                              return next;
                            });
                          }
                        }}
                      >
                        Remove gateway endpoint
                      </button>
                    )}
                    <p className="settings-help">
                      Saving or removing an endpoint revokes prior account
                      grants. Save a key for the reviewed endpoint before using
                      this account.{" "}
                      {profile.cli === "codex"
                        ? "Codex requires a Responses endpoint to preserve its opaque native reasoning and conversation context."
                        : `The CLI uses ${selected.gatewayProtocol}. Other protocols support text and function tool conversion; unsupported native features or context are rejected before sending to preserve history.`}
                    </p>
                  </form>
                )}
                {((selected.managedTurns && selected.apiKeyLabel) ||
                  profile.gatewayProvider) && (
                  <form
                    className="router-create"
                    onSubmit={async (event) => {
                      event.preventDefault();
                      if (profile.authState === "pending_remove") return;
                      if (
                        (profile.gatewayProvider &&
                          (
                            endpoints[profile.id] ??
                            profile.gatewayProvider.baseUrl
                          ).trim() !== profile.gatewayProvider.baseUrl) ||
                        (profile.gatewayProvider &&
                          (protocols[profile.id] ??
                            profile.gatewayProvider.protocol) !==
                            profile.gatewayProvider.protocol)
                      )
                        return;
                      const key = keys[profile.id]?.trim();
                      if (
                        key &&
                        (await command("cli_profile_set_api_key", {
                          profileId: profile.id,
                          expectedRevision: profile.revision,
                          key,
                        }))
                      )
                        setKeys((current) => ({
                          ...current,
                          [profile.id]: "",
                        }));
                    }}
                  >
                    <label htmlFor={`router-key-${profile.id}`}>
                      {profile.gatewayProvider
                        ? "Gateway API key"
                        : selected.apiKeyLabel || "API key"}
                      {!profile.gatewayProvider &&
                        selected.profileTerminal !== false &&
                        " (optional)"}
                    </label>
                    <input
                      id={`router-key-${profile.id}`}
                      type="password"
                      autoComplete="off"
                      disabled={busy || profile.authState === "pending_remove"}
                      value={keys[profile.id] ?? ""}
                      onChange={(event) =>
                        setKeys((current) => ({
                          ...current,
                          [profile.id]: event.target.value,
                        }))
                      }
                    />
                    <button
                      className="button"
                      disabled={
                        busy ||
                        profile.authState === "pending_remove" ||
                        !keys[profile.id]?.trim() ||
                        (!!profile.gatewayProvider &&
                          (protocols[profile.id] ??
                            profile.gatewayProvider.protocol) !==
                            profile.gatewayProvider.protocol) ||
                        (!!profile.gatewayProvider &&
                          (
                            endpoints[profile.id] ??
                            profile.gatewayProvider.baseUrl
                          ).trim() !== profile.gatewayProvider.baseUrl)
                      }
                    >
                      Save API key
                    </button>
                  </form>
                )}
              </SettingRow>
            ))}
            {!profiles.length && (
              <p className="settings-help">No accounts yet.</p>
            )}
            <form
              className="router-create"
              onSubmit={async (event) => {
                event.preventDefault();
                if (
                  poolLabel.trim() &&
                  (await mutate({
                    type: "create_router",
                    cli,
                    label: poolLabel.trim(),
                    orderedProfileIds: profiles.map((item) => item.id),
                  }))
                )
                  setPoolLabel("");
              }}
            >
              <label htmlFor="router-pool-label">Router name</label>
              <input
                id="router-pool-label"
                required
                maxLength={120}
                value={poolLabel}
                onChange={(event) => setPoolLabel(event.target.value)}
              />
              <button
                className="button"
                disabled={busy || !poolLabel.trim() || !profiles.length}
              >
                Create router
              </button>
            </form>
          </>
        )}
      </SettingsSection>
      {selected?.canCreateProfile && (
        <SettingsNotice>
          {selected.gatewayTerminal
            ? "Configure compatible API endpoints and distinct API key bindings for your approved Gateway account pool. Codex requires Responses endpoints. Separate keys do not prove separate billing budgets. Subscription credentials are not Gateway API keys."
            : cli === "codex"
              ? "Before enabling a router, refresh quota for at least two verified accounts with distinct authenticated identities."
              : "Before enabling a router, configure at least two distinct API key bindings. Separate keys do not prove separate billing budgets."}
        </SettingsNotice>
      )}
      {routers.map((router) => (
        <SettingsSection
          key={router.id}
          title={router.label}
          description="Accounts are selected by order at each new launch. Fresh quota balancing can adjust selection. Managed failures require explicit continuation."
        >
          <SettingRow label="Router enabled">
            <input
              aria-label={`Enable ${router.label}`}
              type="checkbox"
              checked={router.enabled}
              disabled={busy}
              onChange={(event) =>
                void mutate({
                  type: "update_router",
                  routerId: router.id,
                  enabled: event.target.checked,
                })
              }
            />
          </SettingRow>
          <SettingRow
            label="Balance remaining quota"
            description="Use fresh quota reports when available. Otherwise, keep account order."
          >
            <input
              aria-label={`Balance remaining quota for ${router.label}`}
              type="checkbox"
              checked={router.balanceRemainingQuota}
              disabled={busy || !selected?.balance}
              onChange={(event) =>
                void mutate({
                  type: "update_router",
                  routerId: router.id,
                  balanceRemainingQuota: event.target.checked,
                })
              }
            />
          </SettingRow>
          {router.balanceRemainingQuota &&
            (!selected?.quotaRead ||
              !hasComparableQuota(
                router,
                profiles,
                snapshot?.quota ?? [],
                now,
              )) && (
              <SettingsNotice>
                Fresh quota reports are unavailable. Runs use account order.
              </SettingsNotice>
            )}
          <ol
            className="router-account-order"
            aria-label={`Account order for ${router.label}`}
          >
            {router.orderedProfileIds.map((profileId, index) => {
              const profile = profiles.find((item) => item.id === profileId);
              return (
                <li key={profileId}>
                  <span>{profile?.label ?? "Unavailable account"}</span>
                  <div className="router-inline-actions">
                    <button
                      className="button"
                      disabled={busy || index === 0}
                      aria-label={`Move ${profile?.label ?? "account"} up`}
                      onClick={() =>
                        void mutate({
                          type: "update_router",
                          routerId: router.id,
                          orderedProfileIds: moveAccount(
                            router.orderedProfileIds,
                            profileId,
                            -1,
                          ),
                        })
                      }
                    >
                      Up
                    </button>
                    <button
                      className="button"
                      disabled={
                        busy || index === router.orderedProfileIds.length - 1
                      }
                      aria-label={`Move ${profile?.label ?? "account"} down`}
                      onClick={() =>
                        void mutate({
                          type: "update_router",
                          routerId: router.id,
                          orderedProfileIds: moveAccount(
                            router.orderedProfileIds,
                            profileId,
                            1,
                          ),
                        })
                      }
                    >
                      Down
                    </button>
                    <button
                      className="button"
                      disabled={busy}
                      aria-label={`Remove ${profile?.label ?? "account"} from ${router.label}`}
                      onClick={() =>
                        void mutate({
                          type: "update_router",
                          routerId: router.id,
                          orderedProfileIds: router.orderedProfileIds.filter(
                            (id) => id !== profileId,
                          ),
                        })
                      }
                    >
                      Remove from router
                    </button>
                  </div>
                </li>
              );
            })}
          </ol>
          {profiles
            .filter((profile) => !router.orderedProfileIds.includes(profile.id))
            .map((profile) => (
              <button
                key={profile.id}
                className="button"
                disabled={busy}
                onClick={() =>
                  void mutate({
                    type: "update_router",
                    routerId: router.id,
                    orderedProfileIds: [
                      ...router.orderedProfileIds,
                      profile.id,
                    ],
                  })
                }
              >
                Add {profile.label}
              </button>
            ))}
          <div className="router-inline-actions">
            <button
              className="button"
              disabled={busy || !selected?.quotaRead}
              onClick={() =>
                void command("cli_router_refresh_quota", {
                  routerId: router.id,
                })
              }
            >
              Refresh quota
            </button>
            <button
              className="button"
              disabled={busy}
              onClick={() =>
                void mutate({ type: "remove_router", routerId: router.id })
              }
            >
              Remove router
            </button>
          </div>
        </SettingsSection>
      ))}
    </div>
  );
}
