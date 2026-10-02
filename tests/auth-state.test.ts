import assert from "node:assert/strict";
import test from "node:test";
import {
  canShareRemotely,
  newestAuthState,
  unavailableState,
} from "../src/auth/model.ts";
import type { AuthState, AuthStatus } from "../src/auth/model.ts";

test("remote sharing requires a confirmed signed-in account and session", () => {
  const signedIn: AuthState = {
    ...unavailableState,
    status: "signed-in",
    user: {
      id: "user",
      displayName: "Lomi User",
      email: "user@example.test",
      githubLogin: null,
      status: "active",
    },
    session: { id: "session", expiresAt: "2030-01-01T00:00:00Z" },
  };
  assert.equal(canShareRemotely(signedIn), true);
  assert.equal(canShareRemotely(null), false);
  assert.equal(canShareRemotely({ ...signedIn, user: null }), false);
  assert.equal(canShareRemotely({ ...signedIn, session: null }), false);
  for (const status of [
    "unavailable",
    "signed-out",
    "authorizing",
    "checking",
    "offline",
    "storage-locked",
    "error",
  ] satisfies AuthStatus[]) {
    assert.equal(canShareRemotely({ ...signedIn, status }), false, status);
  }
});

test("a delayed IPC reply cannot restore an account after a newer logout event", () => {
  const signedIn = {
    ...unavailableState,
    revision: 7,
    status: "signed-in" as const,
  };
  const signedOut = {
    ...unavailableState,
    revision: 8,
    status: "signed-out" as const,
  };
  assert.equal(newestAuthState(signedOut, signedIn), signedOut);
  assert.equal(newestAuthState(signedIn, signedOut), signedOut);
  assert.equal(
    newestAuthState(signedOut, { ...signedIn, revision: 8 }),
    signedOut,
  );
});

test("invalid event revisions are ignored while the initial native revision is accepted", () => {
  assert.equal(newestAuthState(null, unavailableState), unavailableState);
  for (const revision of [-1, NaN, Infinity, 1.5]) {
    assert.equal(
      newestAuthState(unavailableState, { ...unavailableState, revision }),
      unavailableState,
    );
  }
});
