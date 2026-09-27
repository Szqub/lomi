import assert from "node:assert/strict";
import test from "node:test";
import { newestAuthState, unavailableState } from "../src/auth/model.ts";

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
