export type AuthStatus =
  | "unavailable"
  | "signed-out"
  | "authorizing"
  | "checking"
  | "signed-in"
  | "offline"
  | "storage-locked"
  | "error";

export type AuthStorage = "persistent" | "session";

export interface AuthState {
  revision: number;
  status: AuthStatus;
  user: {
    id: string;
    displayName: string;
    email: string;
    githubLogin: string | null;
    status: string;
  } | null;
  session: { id: string; expiresAt: string } | null;
  attempt: {
    id: string;
    expiresAt: string;
  } | null;
  storage: AuthStorage | null;
  message: string | null;
  remoteRevocationConfirmed: boolean | null;
}

export const unavailableState: AuthState = {
  revision: 0,
  status: "unavailable",
  user: null,
  session: null,
  attempt: null,
  storage: null,
  message: null,
  remoteRevocationConfirmed: null,
};

export function newestAuthState(
  current: AuthState | null,
  incoming: AuthState,
): AuthState | null {
  if (
    !Number.isSafeInteger(incoming.revision) ||
    incoming.revision < 0 ||
    (current !== null && incoming.revision <= current.revision)
  ) {
    return current;
  }
  return incoming;
}

export function authStatusLabel(status: AuthStatus): string {
  switch (status) {
    case "unavailable":
      return "Unavailable";
    case "signed-out":
      return "Signed out";
    case "authorizing":
      return "Waiting for browser";
    case "checking":
      return "Checking account…";
    case "signed-in":
      return "Signed in";
    case "offline":
      return "Connection unavailable";
    case "storage-locked":
      return "Credential store locked";
    case "error":
      return "Needs attention";
  }
}
