export type GatewayProtocol =
  "anthropic" | "openai_chat" | "openai_responses" | "gemini";
export interface GatewayProvider {
  protocol: GatewayProtocol;
  baseUrl: string;
}

export interface CliProfile {
  id: string;
  cli: string;
  label: string;
  enabled: boolean;
  revision: number;
  authState:
    | "disconnected"
    | "connecting"
    | "verifying"
    | "ready"
    | "refreshing"
    | "reauth_required"
    | "disabled"
    | "pending_remove"
    | "identity_mismatch"
    | "error"
    | "unverified";
  storageMode: "keyring" | "session_only" | "cli_managed" | "api_key";
  gatewayProvider?: GatewayProvider | null;
}
export interface CliRouter {
  id: string;
  cli: string;
  label: string;
  enabled: boolean;
  orderedProfileIds: string[];
  balanceRemainingQuota: boolean;
  revision: number;
}
export interface CliQuota {
  profileId: string;
  status:
    | "unknown"
    | "fresh"
    | "stale"
    | "exhausted"
    | "reader_throttled"
    | "reader_error"
    | "unsupported";
  windows: {
    id: string;
    remainingPercent: number | null;
    resetAt: number | null;
  }[];
  observedAt: number;
  expiresAt: number;
  epoch: number;
  blockRevision: number;
}
export type RunState =
  | "idle"
  | "starting"
  | "running"
  | "switching"
  | "waiting_for_capacity"
  | "paused"
  | "completed"
  | "stopped"
  | "failed"
  | "recovery_required";
export interface CliRun {
  id: string;
  routerId: string;
  cwd: string;
  shellProfileId: string | null;
  title: string;
  state: RunState;
  model: string | null;
  reasoningEffort?: string | null;
  executionMode?: "text" | "coding" | "gateway" | "native";
  pinnedProfileId: string | null;
  allowedProfileIds: string[];
  activeProfileId: string | null;
  generation: number;
  revision: number;
  inputs: { id: string; text: string }[];
  attempts: {
    id: string;
    inputId: string;
    profileId: string;
    generation: number;
    state: string;
    reason: string;
  }[];
  turns?: {
    inputId: string;
    attemptId: string;
    profileId: string;
    generation: number;
    state: string;
    text: string;
  }[];
  legacyOutput?: string | null;
  output: string;
  statusMessage: string;
  attemptedProfileIds: string[];
}
export interface CliRouterCapability {
  cli: string;
  name: string;
  canCreateProfile: boolean;
  canVerifyLogin: boolean;
  profileTerminal?: boolean;
  canStart: boolean;
  managedTurns: boolean;
  codingTurns?: boolean;
  nativeTurns?: boolean;
  nativeAccountTerminal?: boolean;
  gatewayTerminal?: boolean;
  nativeHistoryResume?: boolean;
  gatewayProtocol?:
    "anthropic" | "openai_chat" | "openai_responses" | "gemini" | null;
  gatewaySource?: string | null;
  gatewayGates?: string | null;
  apiKeyLabel?: string | null;
  models?: { id: string; reasoningEfforts?: string[] }[];
  sameAccountResume: boolean;
  crossAccountResume: boolean;
  quotaRead: boolean;
  balance: boolean;
  readOnly: boolean;
  reason: string;
}
export interface CliRouterSnapshot {
  revision: number;
  profiles: CliProfile[];
  routers: CliRouter[];
  quota: CliQuota[];
  runs: CliRun[];
  capabilities: CliRouterCapability[];
}
export type RouterAction =
  | {
      type: "configure_gateway_account";
      profileId: string;
      provider: GatewayProvider | null;
    }
  | { type: "create_profile"; cli: string; label: string }
  | {
      type: "update_profile";
      profileId: string;
      label?: string;
      enabled?: boolean;
    }
  | { type: "remove_profile"; profileId: string }
  | {
      type: "create_router";
      cli: string;
      label: string;
      orderedProfileIds: string[];
      enabled?: boolean;
      balanceRemainingQuota?: boolean;
    }
  | {
      type: "update_router";
      routerId: string;
      label?: string;
      enabled?: boolean;
      balanceRemainingQuota?: boolean;
      orderedProfileIds?: string[];
    }
  | { type: "remove_router"; routerId: string };
