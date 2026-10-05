import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { cliNames, type CliAgent } from "../cli-agents";
import { CliAgentIcon } from "../CliAgentIcon";
import {
  ArrowLeft,
  ArrowUp,
  ArrowDown,
  Check,
  ChevronRight,
  Info,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Terminal,
  Trash2,
} from "../icons";
import { SettingRow, SettingsNotice, SettingsSection } from "../settings-ui";
import { DisclosureSummary, IconButton, Modal } from "../ui";
import Select from "../Select";
import {
  moveAccount,
  quotaLabel,
  isQuotaFresh,
  hasComparableQuota,
  gatewayProtocolsForCli,
} from "./model";
import type { CliProfile, CliRouter, GatewayProtocol } from "./types";
import { useRouterSnapshot } from "./useRouterSnapshot";
import "./router.css";
import "./router-settings.css";

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
const authLabels = {
  disconnected: "Not signed in",
  connecting: "Signing in…",
  verifying: "Verifying…",
  ready: "Verified",
  refreshing: "Refreshing…",
  reauth_required: "Sign in again",
  disabled: "Disabled",
  pending_remove: "Removal pending",
  identity_mismatch: "Account changed",
  error: "Needs attention",
  unverified: "Not verified",
};
const featuredClis: CliAgent[] = [
  "claude",
  "codex",
  "grok",
  "kimi",
  "kilo",
  "opencode",
  "pi",
  "agy",
];
type RouteDraft = {
  id?: string;
  revision?: number;
  label: string;
  ids: string[];
  balance: boolean;
  cli: string;
  enabled: boolean;
};
function canonicalUrl(value: string) {
  try {
    return new URL(value).toString().replace(/\/+$/, "");
  } catch {
    return value;
  }
}
function NativeAccountReport({
  report,
  now,
}: {
  report: NativeReport;
  now: number;
}) {
  return (
    <div className="router-account-diagnostics">
      <p className="setting-value">
        {report.authenticated ? "Signed in" : "Not verified"}
        {report.observation.identity?.displayLabel &&
          ` · ${report.observation.identity.displayLabel}`}
        {report.expiresAt <= now && " · Out of date"}
      </p>
      {report.error && (
        <SettingsNotice tone="error">{report.error}</SettingsNotice>
      )}
      <div className="router-account-limits">
        {report.observation.windows.map((window) => (
          <SettingRow
            key={window.id}
            label={window.name}
            description={`${window.nativeWindow || "Scope unknown"}${window.resetAt != null ? ` · Resets ${new Date(window.resetAt).toLocaleString()}` : ""}`}
          >
            <span className="setting-value">
              {window.disabled
                ? "Disabled"
                : window.remainingPercent != null &&
                    Number.isFinite(window.remainingPercent)
                  ? `${window.remainingPercent.toFixed(1)}% remaining`
                  : window.remainingAmount != null &&
                      Number.isFinite(window.remainingAmount)
                    ? `${window.remainingAmount} ${window.unit || "(unit unknown)"} remaining`
                    : "Quota unknown"}
            </span>
          </SettingRow>
        ))}
      </div>
      <p className="settings-help">{`Updated ${new Date(report.observedAt).toLocaleTimeString()}. Reported limits; model scope is unverified for balancing.`}</p>
    </div>
  );
}
export default function RouterSettings() {
  const { snapshot, error, busy, mutate, command, clearError, readSnapshot } =
    useRouterSnapshot();
  const [now, setNow] = useState(Date.now);
  const [search, setSearch] = useState("");
  const [choosingCli, setChoosingCli] = useState(false);
  const [support, setSupport] = useState(false);
  const [adding, setAdding] = useState(false);
  const [accountLabel, setAccountLabel] = useState("");
  const [accountId, setAccountId] = useState<string>();
  const [connection, setConnection] = useState<"cli" | "api">("cli");
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [endpoints, setEndpoints] = useState<Record<string, string>>({});
  const [protocols, setProtocols] = useState<Record<string, GatewayProtocol>>(
    {},
  );
  const [routeDraft, setRouteDraft] = useState<RouteDraft>();
  const [removing, setRemoving] = useState<CliProfile>();
  const [removingRouter, setRemovingRouter] = useState(false);
  const [nativeReports, setNativeReports] = useState<
    Record<string, NativeReport>
  >({});
  const [nativeBusy, setNativeBusy] = useState<string>();
  const [localError, setLocalError] = useState("");
  const [connecting, setConnecting] = useState(false);
  const connectionLock = useRef(false);
  const nameInput = useRef<HTMLInputElement>(null);
  const routerNameInput = useRef<HTMLInputElement>(null);
  const agentSearch = useRef<HTMLInputElement>(null);
  const cancelRemove = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!routeDraft) return;
    if (choosingCli) agentSearch.current?.focus();
    else routerNameInput.current?.focus();
  }, [choosingCli, routeDraft?.cli, routeDraft?.id]);
  const cli = routeDraft?.cli ?? "";
  const capabilities = snapshot?.capabilities ?? [];
  const selected = capabilities.find((item) => item.cli === cli);
  const profiles = snapshot?.profiles.filter((item) => item.cli === cli) ?? [];
  const routers = snapshot?.routers ?? [];
  const account = profiles.find((item) => item.id === accountId);
  const locked = busy || connecting || !!nativeBusy;
  const apiSupported =
    !!(selected?.gatewayTerminal && selected.gatewayProtocol) ||
    !!(selected?.managedTurns && selected.apiKeyLabel);
  const cliSupported = selected?.profileTerminal !== false;
  const directApiSupported = !!(selected?.managedTurns && selected.apiKeyLabel);
  const hasFreshReports =
    !!snapshot?.quota.some((quota) => isQuotaFresh(quota, now)) ||
    Object.values(nativeReports).some((report) => report.expiresAt > now);
  useEffect(() => setNow(Date.now()), [snapshot]);
  useEffect(() => {
    if (!hasFreshReports) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [hasFreshReports]);
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
      if (live)
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
    if (locked) return;
    setNativeBusy(profileId);
    setLocalError("");
    try {
      const report = await invoke<NativeReport>("cli_profile_refresh_native", {
        profileId,
      });
      setNativeReports((current) => ({ ...current, [profileId]: report }));
      await command("cli_router_snapshot", {});
    } catch (failure) {
      setLocalError(String(failure));
    } finally {
      setNativeBusy(undefined);
    }
  }
  function openAccount(profile: CliProfile) {
    clearError();
    setConnection(
      profile.gatewayProvider ||
        profile.storageMode === "api_key" ||
        !cliSupported
        ? "api"
        : "cli",
    );
    setLocalError("");
    setAccountId(profile.id);
  }
  function clearConnectionDraft(profileId: string) {
    setKeys((current) => ({ ...current, [profileId]: "" }));
    setEndpoints((current) => {
      const next = { ...current };
      delete next[profileId];
      return next;
    });
    setProtocols((current) => {
      const next = { ...current };
      delete next[profileId];
      return next;
    });
  }
  const endpoint = account
    ? (endpoints[account.id] ?? account.gatewayProvider?.baseUrl ?? "")
    : "";
  const protocol = (
    account?.cli === "codex"
      ? "openai_responses"
      : account &&
        (protocols[account.id] ??
          account.gatewayProvider?.protocol ??
          selected?.gatewayProtocol)
  ) as GatewayProtocol | undefined;
  async function saveConnection(endpointOnly = false) {
    if (
      !account ||
      locked ||
      connectionLock.current ||
      account.authState === "pending_remove"
    )
      return;
    const reviewed = account;
    const key = keys[reviewed.id]?.trim();
    const destination = endpoint.trim();
    if (!endpointOnly && !key) return;
    const gatewayDestination =
      !!selected?.gatewayTerminal &&
      (!!destination || !!reviewed.gatewayProvider || !directApiSupported);
    if (gatewayDestination && (!destination || !protocol)) return;
    connectionLock.current = true;
    setConnecting(true);
    setLocalError("");
    try {
      let binding = reviewed;
      if (
        gatewayDestination &&
        protocol &&
        (endpointOnly ||
          reviewed.gatewayProvider?.protocol !== protocol ||
          reviewed.gatewayProvider?.baseUrl !== destination)
      ) {
        const next = await mutate({
          type: "configure_gateway_account",
          profileId: reviewed.id,
          provider: { protocol, baseUrl: destination },
        });
        if (!next) return;
        const updated = next.profiles.find(
          (profile) => profile.id === reviewed.id,
        );
        // The key is bound to the destination returned by the reviewed mutation.
        if (
          !updated ||
          updated.gatewayProvider?.protocol !== protocol ||
          canonicalUrl(updated.gatewayProvider.baseUrl) !==
            canonicalUrl(destination) ||
          updated.authState === "pending_remove"
        ) {
          setLocalError(
            "The account destination changed. Review it before saving a key.",
          );
          return;
        }
        binding = updated;
        if (endpointOnly) {
          clearConnectionDraft(reviewed.id);
          return;
        }
      }
      if (
        key &&
        (await command("cli_profile_set_api_key", {
          profileId: binding.id,
          expectedRevision: binding.revision,
          key,
        }))
      ) {
        clearConnectionDraft(binding.id);
        setAccountId(undefined);
      }
    } finally {
      connectionLock.current = false;
      setConnecting(false);
    }
  }
  function clearDraftError() {
    setLocalError("");
    clearError();
  }
  function closeRoute() {
    setRouteDraft(undefined);
    setChoosingCli(false);
    setSearch("");
    clearDraftError();
  }
  function openRoute(router?: CliRouter) {
    clearDraftError();
    setSearch("");
    setChoosingCli(!router);
    setRouteDraft(
      router
        ? {
            id: router.id,
            revision: router.revision,
            cli: router.cli,
            enabled: router.enabled,
            label: router.label,
            ids: [...router.orderedProfileIds],
            balance: router.balanceRemainingQuota,
          }
        : { cli: "", enabled: false, label: "", ids: [], balance: false },
    );
  }
  function chooseCli(nextCli: string) {
    const capability = capabilities.find((item) => item.cli === nextCli);
    const previousDefault = `${selected?.name || cliNames[cli as CliAgent] || cli} router`;
    setRouteDraft((draft) =>
      draft && !draft.id
        ? {
            ...draft,
            cli: nextCli,
            label:
              (draft.label !== previousDefault && draft.label) ||
              `${capability?.name || cliNames[nextCli as CliAgent] || nextCli} router`,
            ids:
              snapshot?.profiles
                .filter(
                  (profile) =>
                    profile.cli === nextCli &&
                    profile.authState !== "pending_remove",
                )
                .map((profile) => profile.id) ?? [],
            balance: false,
            enabled: false,
          }
        : draft,
    );
    setChoosingCli(false);
    setSearch("");
    clearDraftError();
  }
  function addAccount() {
    setAccountLabel("");
    setAdding(true);
    setConnection(cliSupported ? "cli" : "api");
    clearDraftError();
  }
  const routeChanged =
    !!routeDraft?.id &&
    routers.find((router) => router.id === routeDraft.id)?.revision !==
      routeDraft.revision;
  const routeMissingAccount = !!routeDraft?.ids.some(
    (id) => !profiles.some((profile) => profile.id === id),
  );
  const modalError = error || localError;
  return (
    <div className="router-settings" aria-busy={locked}>
      {!account && !adding && !routeDraft && !removing && modalError && (
        <SettingsNotice tone="error">{modalError}</SettingsNotice>
      )}
      {!snapshot && !error && <p role="status">Loading routers…</p>}
      {snapshot && (
        <SettingsSection
          title="Saved routers"
          count={routers.length || undefined}
          actions={
            <button
              type="button"
              className="button button-primary"
              disabled={locked}
              onClick={() => openRoute()}
            >
              <Plus size={14} aria-hidden="true" /> New router
            </button>
          }
        >
          {!routers.length ? (
            <div className="router-dashboard-empty">
              <p>No routers yet</p>
              <span className="settings-help">
                Create a router to choose an agent and its accounts.
              </span>
            </div>
          ) : (
            <div className="router-compact-list">
              {routers.map((router) => {
                const capability = capabilities.find(
                  (item) => item.cli === router.cli,
                );
                const fallback =
                  router.balanceRemainingQuota &&
                  (!capability?.quotaRead ||
                    !hasComparableQuota(
                      router,
                      snapshot.profiles,
                      snapshot.quota,
                      now,
                    ));
                return (
                  <div className="router-compact-row" key={router.id}>
                    <button
                      type="button"
                      className="router-row-content"
                      disabled={locked}
                      aria-label={`Manage ${router.label}`}
                      onClick={() => openRoute(router)}
                    >
                      <CliAgentIcon cli={router.cli as CliAgent} />
                      <span className="router-row-name">
                        {router.label}
                        <small>
                          {capability?.name ||
                            cliNames[router.cli as CliAgent] ||
                            router.cli}{" "}
                          · {router.orderedProfileIds.length}{" "}
                          {router.orderedProfileIds.length === 1
                            ? "account"
                            : "accounts"}
                        </small>
                      </span>
                      {fallback && (
                        <span
                          className="router-fallback"
                          title="No comparable fresh quota. Account order applies."
                        >
                          Account order
                        </span>
                      )}
                      <Settings size={15} aria-hidden="true" />
                    </button>
                    <input
                      className="settings-switch"
                      type="checkbox"
                      role="switch"
                      aria-label={`Enable ${router.label}`}
                      checked={router.enabled}
                      disabled={locked}
                      onChange={(event) =>
                        void mutate({
                          type: "update_router",
                          routerId: router.id,
                          enabled: event.target.checked,
                        })
                      }
                    />
                  </div>
                );
              })}
            </div>
          )}
        </SettingsSection>
      )}
      {support && selected && (
        <Modal
          title={selected.name}
          className="router-settings-dialog"
          onClose={() => setSupport(false)}
        >
          <div className="router-setup-body">
            <p>
              {selected.reason ||
                "Separate accounts are unavailable for this CLI."}
            </p>
            {selected.gatewayTerminal && (
              <p>
                API keys use the system keyring. Subscription login stays in the
                CLI. Native CLI permission prompts apply.
              </p>
            )}
            {selected.gatewayTerminal && (
              <p>
                {selected.cli === "codex"
                  ? "API endpoints must support OpenAI Responses to preserve native context."
                  : "Protocol conversion supports text and function tools. Unsupported native features or context are rejected before sending."}
              </p>
            )}
            <p>
              Enable a router after configuring two distinct, compatible
              accounts. Separate API keys may share a billing budget.
            </p>
          </div>
        </Modal>
      )}
      {adding && selected && (
        <Modal
          title="Add account"
          className="router-settings-dialog"
          initialFocus={nameInput}
          closeDisabled={locked}
          onClose={() => setAdding(false)}
        >
          <form
            onSubmit={async (event) => {
              event.preventDefault();
              if (locked || !accountLabel.trim()) return;
              const draftAtCreation = routeDraft;
              const next = await mutate({
                type: "create_profile",
                cli,
                label: accountLabel.trim(),
              });
              const created = next?.profiles.find(
                (profile) =>
                  profile.cli === cli &&
                  !profiles.some((item) => item.id === profile.id),
              );
              if (created) {
                setRouteDraft((draft) =>
                  draft &&
                  draft === draftAtCreation &&
                  draft.cli === created.cli
                    ? { ...draft, ids: [...draft.ids, created.id] }
                    : draft,
                );
                setAdding(false);
                setAccountId(created.id);
              }
            }}
          >
            <div className="router-setup-body">
              {modalError && (
                <SettingsNotice tone="error">{modalError}</SettingsNotice>
              )}
              <label className="router-setup-field">
                Account name
                <input
                  ref={nameInput}
                  required
                  maxLength={120}
                  value={accountLabel}
                  disabled={locked}
                  placeholder="Personal, work…"
                  onChange={(event) => setAccountLabel(event.target.value)}
                />
              </label>
              <p className="settings-help">
                Accounts are saved separately and can be reused by other
                routers.
              </p>
              {apiSupported && cliSupported && (
                <div
                  className="router-connection-choices"
                  role="group"
                  aria-label="Connection type"
                >
                  <button
                    type="button"
                    className="button"
                    aria-pressed={connection === "cli"}
                    disabled={locked}
                    onClick={() => setConnection("cli")}
                  >
                    <Terminal size={16} aria-hidden="true" />
                    CLI login
                  </button>
                  <button
                    type="button"
                    className="button"
                    aria-pressed={connection === "api"}
                    disabled={locked}
                    onClick={() => setConnection("api")}
                  >
                    <Plus size={16} aria-hidden="true" />
                    API key
                  </button>
                </div>
              )}
            </div>
            <div className="router-setup-footer">
              <button
                type="button"
                className="button"
                disabled={locked}
                onClick={() => setAdding(false)}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={locked || !accountLabel.trim()}
              >
                Add account
              </button>
            </div>
          </form>
        </Modal>
      )}
      {account && selected && (
        <Modal
          title={account.label}
          className="router-settings-dialog"
          closeDisabled={locked}
          onClose={() => {
            setAccountId(undefined);
            setLocalError("");
          }}
        >
          <div className="router-setup-body">
            {modalError && (
              <SettingsNotice tone="error">{modalError}</SettingsNotice>
            )}
            <SettingRow label="Enable account" htmlFor="router-account-enabled">
              <input
                id="router-account-enabled"
                className="settings-switch"
                type="checkbox"
                role="switch"
                aria-label={`Enable ${account.label}`}
                checked={account.enabled}
                disabled={locked || account.authState === "pending_remove"}
                onChange={(event) =>
                  void mutate({
                    type: "update_profile",
                    profileId: account.id,
                    enabled: event.target.checked,
                  })
                }
              />
            </SettingRow>
            {apiSupported && cliSupported && (
              <div
                className="router-connection-choices"
                role="group"
                aria-label="Connection type"
              >
                <button
                  type="button"
                  className="button"
                  aria-pressed={connection === "cli"}
                  disabled={locked}
                  onClick={() => setConnection("cli")}
                >
                  CLI login
                </button>
                <button
                  type="button"
                  className="button"
                  aria-pressed={connection === "api"}
                  disabled={locked}
                  onClick={() => setConnection("api")}
                >
                  API key
                </button>
              </div>
            )}
            {connection === "cli" ? (
              <>
                <div className="router-login-status">
                  <Terminal size={28} aria-hidden="true" />
                  <span>{authLabels[account.authState]}</span>
                </div>
                <button
                  type="button"
                  className="button button-primary router-login-action"
                  disabled={
                    locked ||
                    account.authState === "pending_remove" ||
                    account.storageMode === "api_key" ||
                    !cliSupported
                  }
                  onClick={() =>
                    void command("cli_profile_open_terminal", {
                      profileId: account.id,
                    })
                  }
                >
                  Sign in with CLI
                </button>
                <div className="router-inline-actions">
                  <button
                    type="button"
                    className="button"
                    disabled={
                      locked ||
                      account.authState === "pending_remove" ||
                      !selected.canVerifyLogin
                    }
                    onClick={() =>
                      void command("cli_profile_verify", {
                        profileId: account.id,
                      })
                    }
                  >
                    <Check size={14} aria-hidden="true" />
                    Verify
                  </button>
                  {account.storageMode === "cli_managed" &&
                    featuredClis.includes(cli as CliAgent) && (
                      <button
                        type="button"
                        className="button"
                        disabled={
                          locked || account.authState === "pending_remove"
                        }
                        onClick={() => void refreshNative(account.id)}
                      >
                        <RefreshCw size={14} aria-hidden="true" />
                        Refresh account status
                      </button>
                    )}
                </div>
                {nativeReports[account.id]?.profileRevision ===
                  account.revision && (
                  <details className="settings-disclosure">
                    <DisclosureSummary>Account details</DisclosureSummary>
                    <div className="settings-disclosure-body">
                      <NativeAccountReport
                        report={nativeReports[account.id]}
                        now={now}
                      />
                    </div>
                  </details>
                )}
              </>
            ) : (
              <form
                id="router-api-connection"
                onSubmit={(event) => {
                  event.preventDefault();
                  void saveConnection();
                }}
              >
                {selected.gatewayTerminal && (
                  <label className="router-setup-field">
                    API endpoint
                    {directApiSupported &&
                      !account.gatewayProvider &&
                      " (optional)"}
                    <input
                      aria-label="API endpoint"
                      type="url"
                      required={
                        !directApiSupported || !!account.gatewayProvider
                      }
                      maxLength={2048}
                      placeholder="https://api.example.com"
                      value={endpoint}
                      disabled={
                        locked || account.authState === "pending_remove"
                      }
                      onChange={(event) =>
                        setEndpoints((current) => ({
                          ...current,
                          [account.id]: event.target.value,
                        }))
                      }
                    />
                  </label>
                )}
                {selected.gatewayTerminal &&
                  directApiSupported &&
                  !account.gatewayProvider && (
                    <p className="settings-help">
                      Leave the endpoint blank to use this CLI’s default
                      provider.
                    </p>
                  )}
                <label className="router-setup-field">
                  {account.gatewayProvider || endpoint.trim()
                    ? "API key"
                    : selected.apiKeyLabel || "API key"}
                  <input
                    type="password"
                    autoComplete="off"
                    value={keys[account.id] ?? ""}
                    disabled={locked || account.authState === "pending_remove"}
                    onChange={(event) =>
                      setKeys((current) => ({
                        ...current,
                        [account.id]: event.target.value,
                      }))
                    }
                  />
                </label>
                {account.gatewayProvider &&
                  (endpoint.trim() !== account.gatewayProvider.baseUrl ||
                    protocol !== account.gatewayProvider.protocol) && (
                    <p className="settings-help">
                      Changing the endpoint revokes this account’s saved run
                      grants.
                    </p>
                  )}
                {selected.gatewayTerminal && (
                  <details className="settings-disclosure">
                    <DisclosureSummary>Advanced</DisclosureSummary>
                    <div className="settings-disclosure-body router-setup-options">
                      <div className="router-setup-field">
                        <label htmlFor="router-api-protocol">
                          API protocol
                        </label>
                        <Select
                          id="router-api-protocol"
                          aria-label="API protocol"
                          value={protocol ?? ""}
                          disabled={
                            locked || account.authState === "pending_remove"
                          }
                          onChange={(value) =>
                            setProtocols((current) => ({
                              ...current,
                              [account.id]: value as GatewayProtocol,
                            }))
                          }
                          options={gatewayProtocolsForCli(cli).map((item) => ({
                            value: item.id,
                            label: item.label,
                          }))}
                        />
                      </div>
                      <div className="router-inline-actions">
                        <button
                          type="button"
                          className="button"
                          disabled={
                            locked ||
                            account.authState === "pending_remove" ||
                            !endpoint.trim()
                          }
                          onClick={() => void saveConnection(true)}
                        >
                          Save endpoint
                        </button>
                        {account.gatewayProvider && (
                          <button
                            type="button"
                            className="button"
                            disabled={
                              locked || account.authState === "pending_remove"
                            }
                            onClick={async () => {
                              if (
                                await mutate({
                                  type: "configure_gateway_account",
                                  profileId: account.id,
                                  provider: null,
                                })
                              ) {
                                clearConnectionDraft(account.id);
                                setEndpoints((current) => ({
                                  ...current,
                                  [account.id]: "",
                                }));
                              }
                            }}
                          >
                            Remove endpoint
                          </button>
                        )}
                      </div>
                      <p className="settings-help">
                        Saving or removing an endpoint revokes account grants
                        and clears the key draft. API keys and subscription
                        logins are separate.
                      </p>
                    </div>
                  </details>
                )}
              </form>
            )}
          </div>
          <div className="router-setup-footer">
            <IconButton
              title={`Remove account ${account.label}`}
              disabled={locked}
              onClick={() => setRemoving(account)}
            >
              <Trash2 size={16} />
            </IconButton>
            <span className="router-footer-spacer" />
            {connection === "api" && (
              <button
                type="button"
                className="button"
                disabled={
                  locked ||
                  account.authState === "pending_remove" ||
                  (!selected.canVerifyLogin &&
                    account.storageMode !== "api_key")
                }
                onClick={() =>
                  void command("cli_profile_verify", { profileId: account.id })
                }
              >
                Verify
              </button>
            )}
            {connection === "api" ? (
              <button
                type="submit"
                form="router-api-connection"
                className="button button-primary"
                disabled={
                  locked ||
                  account.authState === "pending_remove" ||
                  !keys[account.id]?.trim() ||
                  (!!selected.gatewayTerminal &&
                    (!directApiSupported || !!account.gatewayProvider) &&
                    !endpoint.trim())
                }
              >
                {connecting ? "Connecting…" : "Save connection"}
              </button>
            ) : (
              <button
                type="button"
                className="button"
                disabled={locked}
                onClick={() => setAccountId(undefined)}
              >
                Done
              </button>
            )}
          </div>
        </Modal>
      )}
      {routeDraft && (
        <Modal
          title={routeDraft.id ? "Edit router" : "New router"}
          className="router-settings-dialog"
          initialFocus={choosingCli ? agentSearch : routerNameInput}
          closeDisabled={locked}
          onClose={closeRoute}
        >
          {choosingCli ? (
            <>
              <div className="router-setup-body router-cli-picker">
                {modalError && (
                  <SettingsNotice tone="error">{modalError}</SettingsNotice>
                )}
                <div className="agent-client-search-field">
                  <Search size={16} aria-hidden="true" />
                  <input
                    ref={agentSearch}
                    type="search"
                    aria-label="Search agents"
                    placeholder="Find a CLI…"
                    value={search}
                    disabled={locked}
                    onChange={(event) => setSearch(event.target.value)}
                  />
                </div>
                <div className="router-cli-list" aria-label="CLI agents">
                  {capabilities
                    .filter((item) =>
                      `${item.name || cliNames[item.cli as CliAgent]} ${item.cli}`
                        .toLowerCase()
                        .includes(search.trim().toLowerCase()),
                    )
                    .map((item) => (
                      <button
                        type="button"
                        className="router-cli-option"
                        key={item.cli}
                        aria-label={
                          item.name ||
                          cliNames[item.cli as CliAgent] ||
                          item.cli
                        }
                        disabled={locked}
                        onClick={() => chooseCli(item.cli)}
                      >
                        <CliAgentIcon cli={item.cli as CliAgent} />
                        <span className="router-cli-name">
                          {item.name ||
                            cliNames[item.cli as CliAgent] ||
                            item.cli}
                        </span>
                        <ChevronRight size={14} aria-hidden="true" />
                      </button>
                    ))}
                </div>
                {!capabilities.some((item) =>
                  `${item.name || cliNames[item.cli as CliAgent]} ${item.cli}`
                    .toLowerCase()
                    .includes(search.trim().toLowerCase()),
                ) && <p className="settings-help">No matching CLI.</p>}
              </div>
              <div className="router-setup-footer">
                <button
                  type="button"
                  className="button"
                  disabled={locked}
                  onClick={closeRoute}
                >
                  Cancel
                </button>
              </div>
            </>
          ) : (
            <form
              onSubmit={async (event) => {
                event.preventDefault();
                if (
                  locked ||
                  routeChanged ||
                  routeMissingAccount ||
                  !routeDraft.label.trim() ||
                  !routeDraft.ids.length ||
                  !selected
                )
                  return;
                const next = await mutate(
                  routeDraft.id
                    ? {
                        type: "update_router",
                        routerId: routeDraft.id,
                        label: routeDraft.label.trim(),
                        enabled: routeDraft.enabled,
                        orderedProfileIds: routeDraft.ids,
                        balanceRemainingQuota: routeDraft.balance,
                      }
                    : {
                        type: "create_router",
                        cli: routeDraft.cli,
                        label: routeDraft.label.trim(),
                        enabled: routeDraft.enabled,
                        orderedProfileIds: routeDraft.ids,
                        balanceRemainingQuota: routeDraft.balance,
                      },
                );
                if (next) closeRoute();
              }}
            >
              <div className="router-setup-body">
                <div className="router-draft-agent">
                  <CliAgentIcon cli={routeDraft.cli as CliAgent} />
                  <span>
                    {selected?.name ||
                      cliNames[routeDraft.cli as CliAgent] ||
                      routeDraft.cli}
                  </span>
                  <IconButton
                    title="CLI support details"
                    disabled={!selected}
                    onClick={() => setSupport(true)}
                  >
                    <Info size={16} />
                  </IconButton>
                  {!routeDraft.id && (
                    <button
                      type="button"
                      className="button"
                      disabled={locked}
                      onClick={() => {
                        setChoosingCli(true);
                        setSearch("");
                        clearDraftError();
                      }}
                    >
                      <ArrowLeft size={14} aria-hidden="true" /> Change CLI
                    </button>
                  )}
                </div>
                {modalError &&
                  !adding &&
                  !account &&
                  !support &&
                  !removingRouter && (
                    <SettingsNotice tone="error">{modalError}</SettingsNotice>
                  )}
                {(routeChanged || routeMissingAccount) && (
                  <SettingsNotice tone="warning">
                    This router changed. Close and reopen it before saving.
                  </SettingsNotice>
                )}
                <label className="router-setup-field">
                  Router name
                  <input
                    ref={routerNameInput}
                    required
                    maxLength={120}
                    value={routeDraft.label}
                    disabled={locked}
                    onChange={(event) =>
                      setRouteDraft({
                        ...routeDraft,
                        label: event.target.value,
                      })
                    }
                  />
                </label>
                <div className="router-accounts-heading">
                  <h3>Accounts</h3>
                  <button
                    type="button"
                    className="button"
                    disabled={locked || !selected?.canCreateProfile}
                    onClick={addAccount}
                  >
                    <Plus size={14} aria-hidden="true" /> Add account
                  </button>
                </div>
                {!profiles.length && (
                  <p className="settings-help">
                    {selected?.canCreateProfile
                      ? "Add an account to configure this router."
                      : "Account routing is unavailable for this agent. See support details."}
                  </p>
                )}
                <ol
                  className="router-route-accounts"
                  aria-label={`Account order for ${routeDraft.label}`}
                >
                  {[
                    ...routeDraft.ids,
                    ...profiles
                      .map((profile) => profile.id)
                      .filter((id) => !routeDraft.ids.includes(id)),
                  ].map((id) => {
                    const profile = profiles.find((item) => item.id === id);
                    const index = routeDraft.ids.indexOf(id);
                    const quota = snapshot?.quota.find(
                      (item) => item.profileId === id,
                    );
                    const remaining = isQuotaFresh(quota, now)
                      ? Math.min(
                          ...quota.windows.map(
                            (window) => window.remainingPercent!,
                          ),
                        )
                      : undefined;
                    return (
                      <li key={id}>
                        <label>
                          <input
                            type="checkbox"
                            aria-label={`Include ${profile?.label || "Unavailable account"}`}
                            checked={index >= 0}
                            disabled={
                              locked ||
                              !profile ||
                              profile.authState === "pending_remove"
                            }
                            onChange={(event) =>
                              setRouteDraft({
                                ...routeDraft,
                                ids: event.target.checked
                                  ? [...routeDraft.ids, id]
                                  : routeDraft.ids.filter(
                                      (item) => item !== id,
                                    ),
                              })
                            }
                          />
                          <span className="router-row-name">
                            {profile?.label || "Unavailable account"}
                            {profile && (
                              <small>
                                {profile.gatewayProvider ||
                                profile.storageMode === "api_key"
                                  ? "API"
                                  : "CLI"}{" "}
                                ·{" "}
                                {profile.enabled
                                  ? authLabels[profile.authState]
                                  : "Disabled"}
                              </small>
                            )}
                          </span>
                        </label>
                        {profile && (
                          <span
                            className="router-quota"
                            aria-label={quotaLabel(quota, now)}
                            title={quotaLabel(quota, now)}
                          >
                            {remaining == null ? "—" : `${remaining}%`}
                          </span>
                        )}
                        {index >= 0 && (
                          <div className="router-order-actions">
                            <IconButton
                              title={`Move ${profile?.label || "account"} up`}
                              disabled={locked || index === 0}
                              onClick={() =>
                                setRouteDraft({
                                  ...routeDraft,
                                  ids: moveAccount(routeDraft.ids, id, -1),
                                })
                              }
                            >
                              <ArrowUp size={14} />
                            </IconButton>
                            <IconButton
                              title={`Move ${profile?.label || "account"} down`}
                              disabled={
                                locked || index === routeDraft.ids.length - 1
                              }
                              onClick={() =>
                                setRouteDraft({
                                  ...routeDraft,
                                  ids: moveAccount(routeDraft.ids, id, 1),
                                })
                              }
                            >
                              <ArrowDown size={14} />
                            </IconButton>
                          </div>
                        )}
                        {profile && (
                          <IconButton
                            title={`Manage ${profile.label}`}
                            disabled={locked}
                            onClick={() => openAccount(profile)}
                          >
                            <Settings size={15} />
                          </IconButton>
                        )}
                      </li>
                    );
                  })}
                </ol>
                {!!profiles.length && (
                  <p className="settings-help">
                    Accounts are tried from top to bottom.
                  </p>
                )}
                <SettingRow
                  label="Enable router"
                  htmlFor="router-draft-enabled"
                >
                  <input
                    id="router-draft-enabled"
                    className="settings-switch"
                    type="checkbox"
                    role="switch"
                    aria-label="Enable router"
                    checked={routeDraft.enabled}
                    disabled={locked}
                    onChange={(event) =>
                      setRouteDraft({
                        ...routeDraft,
                        enabled: event.target.checked,
                      })
                    }
                  />
                </SettingRow>
                <details className="settings-disclosure">
                  <DisclosureSummary>Advanced</DisclosureSummary>
                  <div className="settings-disclosure-body router-setup-options">
                    <SettingRow
                      label="Balance remaining quota"
                      htmlFor="router-balance-quota"
                    >
                      <input
                        id="router-balance-quota"
                        className="settings-switch"
                        type="checkbox"
                        role="switch"
                        aria-label={`Balance remaining quota for ${routeDraft.label}`}
                        checked={routeDraft.balance}
                        disabled={locked || !selected?.balance}
                        onChange={(event) =>
                          setRouteDraft({
                            ...routeDraft,
                            balance: event.target.checked,
                          })
                        }
                      />
                    </SettingRow>
                    {routeDraft.balance && (
                      <p className="settings-help">
                        Uses comparable fresh reports; otherwise follows account
                        order.
                      </p>
                    )}
                    {routeDraft.id && (
                      <button
                        type="button"
                        className="button"
                        disabled={locked || !selected?.quotaRead}
                        onClick={() =>
                          void command("cli_router_refresh_quota", {
                            routerId: routeDraft.id,
                          })
                        }
                      >
                        <RefreshCw size={14} aria-hidden="true" /> Refresh quota
                      </button>
                    )}
                  </div>
                </details>
              </div>
              <div className="router-setup-footer">
                {routeDraft.id && (
                  <IconButton
                    title={`Remove router ${routeDraft.label}`}
                    disabled={locked || routeChanged}
                    onClick={() => {
                      clearDraftError();
                      setRemovingRouter(true);
                    }}
                  >
                    <Trash2 size={16} />
                  </IconButton>
                )}
                <span className="router-footer-spacer" />
                <button
                  type="button"
                  className="button"
                  disabled={locked}
                  onClick={closeRoute}
                >
                  Cancel
                </button>
                <button
                  className="button button-primary"
                  disabled={
                    locked ||
                    routeChanged ||
                    routeMissingAccount ||
                    !routeDraft.label.trim() ||
                    !routeDraft.ids.length ||
                    !selected
                  }
                >
                  {routeDraft.id ? "Save changes" : "Create router"}
                </button>
              </div>
            </form>
          )}
        </Modal>
      )}
      {removingRouter && routeDraft?.id && (
        <Modal
          title={`Remove ${routeDraft.label}?`}
          className="router-settings-dialog"
          role="alertdialog"
          tone="danger"
          protectTheme
          initialFocus={cancelRemove}
          closeDisabled={locked}
          onClose={() => {
            setRemovingRouter(false);
            clearDraftError();
          }}
        >
          <div className="router-setup-body">
            {modalError && (
              <SettingsNotice tone="error">{modalError}</SettingsNotice>
            )}
            <p>Remove this router? Accounts and run history will stay.</p>
            {routeChanged && (
              <SettingsNotice tone="warning">
                This router changed. Close and review it again.
              </SettingsNotice>
            )}
          </div>
          <div className="router-setup-footer">
            <button
              ref={cancelRemove}
              type="button"
              className="button"
              disabled={locked}
              onClick={() => {
                setRemovingRouter(false);
                clearDraftError();
              }}
            >
              Cancel
            </button>
            <button
              type="button"
              className="button button-danger"
              disabled={locked || routeChanged}
              onClick={async () => {
                if (
                  await mutate({
                    type: "remove_router",
                    routerId: routeDraft.id!,
                  })
                ) {
                  setRemovingRouter(false);
                  closeRoute();
                }
              }}
            >
              Remove router
            </button>
          </div>
        </Modal>
      )}
      {removing && (
        <Modal
          title={`Remove ${removing.label}?`}
          className="router-settings-dialog"
          role="alertdialog"
          tone="danger"
          protectTheme
          initialFocus={cancelRemove}
          closeDisabled={locked}
          onClose={() => setRemoving(undefined)}
        >
          <div className="router-setup-body">
            {modalError && (
              <SettingsNotice tone="error">{modalError}</SettingsNotice>
            )}
            <p>
              The saved key will be deleted. CLI login files stay; sign out in
              the CLI first.
            </p>
            {profiles.find((profile) => profile.id === removing.id)
              ?.revision !== removing.revision && (
              <SettingsNotice tone="warning">
                This account changed. Close and review it again.
              </SettingsNotice>
            )}
          </div>
          <div className="router-setup-footer">
            <button
              ref={cancelRemove}
              type="button"
              className="button"
              disabled={locked}
              onClick={() => setRemoving(undefined)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="button button-danger"
              disabled={
                locked ||
                profiles.find((profile) => profile.id === removing.id)
                  ?.revision !== removing.revision
              }
              onClick={async () => {
                const reviewedDraft = routeDraft;
                const reviewedRouter = snapshot?.routers.find(
                  (router) => router.id === reviewedDraft?.id,
                );
                const next = await mutate({
                  type: "remove_profile",
                  profileId: removing.id,
                });
                const confirmed = next ?? readSnapshot();
                const remaining = confirmed?.profiles.find(
                  (profile) => profile.id === removing.id,
                );
                // Credential cleanup can fail after membership is revoked.
                if (
                  confirmed &&
                  (next ||
                    !remaining ||
                    remaining.authState === "pending_remove")
                ) {
                  setRouteDraft((draft) => {
                    if (!draft || draft !== reviewedDraft) return draft;
                    const ids = draft.ids.filter((id) => id !== removing.id);
                    const updated = confirmed.routers.find(
                      (router) => router.id === draft.id,
                    );
                    const expectedIds =
                      reviewedRouter?.orderedProfileIds.filter(
                        (id) => id !== removing.id,
                      );
                    // Only adopt the revision for this exact local revocation.
                    // Other changes must still invalidate the reviewed draft.
                    const localRevocation =
                      reviewedRouter &&
                      reviewedRouter.revision === draft.revision &&
                      updated &&
                      updated.revision === reviewedRouter.revision + 1 &&
                      updated.cli === reviewedRouter.cli &&
                      updated.label === reviewedRouter.label &&
                      updated.balanceRemainingQuota ===
                        reviewedRouter.balanceRemainingQuota &&
                      updated.enabled ===
                        (reviewedRouter.enabled && !!expectedIds?.length) &&
                      JSON.stringify(updated.orderedProfileIds) ===
                        JSON.stringify(expectedIds);
                    return {
                      ...draft,
                      ids,
                      revision: localRevocation
                        ? updated.revision
                        : draft.revision,
                    };
                  });
                }
                if (next) {
                  setRemoving(undefined);
                  setAccountId(undefined);
                  clearConnectionDraft(removing.id);
                }
              }}
            >
              Remove account
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}
