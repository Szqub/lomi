import type {
  CliRouterSnapshot,
  CliQuota,
  CliProfile,
  CliRouter,
  RunState,
  GatewayProtocol,
} from "./types";

export const gatewayProtocols: { id: GatewayProtocol; label: string }[] = [
  { id: "anthropic", label: "Anthropic Messages" },
  { id: "openai_chat", label: "OpenAI Chat Completions" },
  { id: "openai_responses", label: "OpenAI Responses" },
  { id: "gemini", label: "Gemini Generate Content" },
];

export function gatewayProtocolsForCli(cli?: string) {
  return cli === "codex"
    ? gatewayProtocols.filter((item) => item.id === "openai_responses")
    : gatewayProtocols;
}

export function compatibleGatewayProtocol(
  native: string,
  upstream?: string,
  cli?: string,
) {
  if (cli === "codex") {
    return native === "openai_responses" && upstream === "openai_responses";
  }
  return (
    gatewayProtocols.some((item) => item.id === native) &&
    gatewayProtocols.some((item) => item.id === upstream)
  );
}
export function acceptSnapshot(
  current: CliRouterSnapshot | undefined,
  next: CliRouterSnapshot,
) {
  return !current || next.revision >= current.revision ? next : current;
}
export function moveAccount(ids: string[], id: string, direction: -1 | 1) {
  const index = ids.indexOf(id),
    target = index + direction;
  if (index < 0 || target < 0 || target >= ids.length) return ids;
  const next = [...ids];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}
function quotaTimeFresh(quota: CliQuota, now: number) {
  return (
    Number.isFinite(now) &&
    Number.isFinite(quota.observedAt) &&
    Number.isFinite(quota.expiresAt) &&
    quota.observedAt <= now &&
    now - quota.observedAt <= 30_000 &&
    quota.expiresAt > now
  );
}
export function isQuotaFresh(
  quota: CliQuota | undefined,
  now = Date.now(),
): quota is CliQuota {
  return (
    !!quota &&
    quota.status === "fresh" &&
    quotaTimeFresh(quota, now) &&
    quota.windows.length > 0 &&
    quota.windows.every(
      (window) =>
        window.remainingPercent !== null &&
        Number.isFinite(window.remainingPercent) &&
        window.remainingPercent >= 0 &&
        window.remainingPercent <= 100,
    )
  );
}
export function hasComparableQuota(
  router: CliRouter,
  profiles: CliProfile[],
  quota: CliQuota[],
  now = Date.now(),
) {
  const eligible = profiles.filter(
    (profile) =>
      profile.enabled &&
      profile.authState === "ready" &&
      profile.cli === router.cli &&
      router.orderedProfileIds.includes(profile.id),
  );
  if (!eligible.length) return false;
  const reports = eligible.map((profile) =>
    quota.find((report) => report.profileId === profile.id),
  );
  return reports.every(
    (report) => isQuotaFresh(report, now) && report.epoch === reports[0]?.epoch,
  );
}
export function quotaLabel(quota: CliQuota | undefined, now = Date.now()) {
  if (!quota) return "Quota unknown";
  if (quota.status !== "fresh")
    return `Quota ${quota.status.replaceAll("_", " ")}`;
  if (!quotaTimeFresh(quota, now)) return "Quota stale";
  if (!isQuotaFresh(quota, now)) return "Quota unknown";
  return `${Math.min(...quota.windows.map((window) => window.remainingPercent!))}% remaining`;
}
export function isRunning(state: RunState) {
  return state === "starting" || state === "running" || state === "switching";
}

export function finalRunIds(
  views: readonly { id: string; runId: string }[],
  panelIds?: ReadonlySet<string>,
) {
  const removed = views.filter((view) => !panelIds || panelIds.has(view.id));
  const remaining = views.filter((view) => panelIds && !panelIds.has(view.id));
  return [...new Set(removed.map((view) => view.runId))].filter(
    (id) => !remaining.some((view) => view.runId === id),
  );
}
