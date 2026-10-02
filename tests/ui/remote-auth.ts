import type { Page } from "@playwright/test";
import { unavailableState, type AuthState } from "../../src/auth/model";

export const remoteSignedOut: AuthState = {
  ...unavailableState,
  revision: 1,
  status: "signed-out",
};

export const remoteSignedIn: AuthState = {
  ...remoteSignedOut,
  revision: 10,
  status: "signed-in",
  storage: "session",
  user: {
    id: "remote-user",
    displayName: "Remote User",
    email: "remote@example.test",
    githubLogin: null,
    status: "active",
  },
  session: { id: "remote-session", expiresAt: "2030-01-01T00:00:00Z" },
};

export async function mockRemoteAccount(
  page: Page,
  state = remoteSignedIn,
  deferInitialRead = false,
) {
  await page.addInitScript(
    ({ state, deferInitialRead }) => {
      const desktop = window as any;
      desktop.__remoteAuthTest = {
        state,
        deferInitialRead,
        failInitialRead: false,
        initialReads: [] as (() => void)[],
      };
      desktop.__authInvoke = async (command: string) => {
        const fixture = desktop.__remoteAuthTest;
        const captured = structuredClone(fixture.state);
        if (command === "auth_get_state" && fixture.deferInitialRead) {
          return new Promise((resolve, reject) => {
            fixture.initialReads.push(() => {
              if (fixture.failInitialRead)
                reject(new Error("Account state unavailable"));
              else resolve(captured);
            });
          });
        }
        return captured;
      };
    },
    { state, deferInitialRead },
  );
}

export async function emitRemoteAccount(page: Page, state: AuthState) {
  await page.evaluate(async (state) => {
    const desktop = window as any;
    desktop.__remoteAuthTest.state = state;
    await desktop.__nativeTest.emitEvent("auth-state-changed", state);
  }, state);
}

export async function remoteMutationCalls(page: Page) {
  return page.evaluate(() =>
    (window as any).__nativeTest.calls.filter((call: any) =>
      ["remote_share_workspace", "remote_revoke_grant"].includes(call.command),
    ),
  );
}
