# Account authentication

Account sign-in is optional. Projects, terminals, local files, plugins, and Chat AI
provider credentials do not depend on the account service. The account controls
live in Settings → Account. The titlebar's Sign In button starts the same browser
flow directly, using persistent storage by default. When a stored session is
locked and no account is available, Sign In opens Settings → Account for recovery.
A fresh sign-in response reporting locked storage also opens that page unless a
newer account state has already arrived. An existing authorization attempt is
reopened instead of replaced. The arrow beside Sign In opens a menu with Settings;
locked storage also exposes Account settings and Sign out even without an account.
The signed-in avatar opens an account
menu with Settings, Account settings, Manage account (the browser portal), and
Sign out (this device). The menu also remains available while the account is
offline. General settings remain available through the Open settings keyboard
shortcut and command picker when signed out.

Before starting sign-in, Rust binds an ephemeral listener to `127.0.0.1` and
creates a random PKCE verifier and state. It sends the S256 challenge, state, and
exact loopback callback URI to `/v1/desktop/start`, then opens the returned authorization
URL in the system browser. After GitHub sign-in and approval, the browser redirects
to the native listener. Rust validates the callback state and exchanges the code
once through `/v1/desktop/exchange`, then verifies the issued session through
`/v1/me`. The local callback page stays pending until the native app has activated
the session and completed the selected storage step. It reports authentication
success only after that point; canceled, stale, unconfirmed, or unsaved sessions
show an error page instead. Successful sign-in brings the main Lomi window to the
front without opening or focusing Settings. The page uses bundled Lomi assets and
makes no remote requests. The browser request is Lomi's first-party handshake and does not use
GitHub's device flow.

Debug and release builds default to the production origin `https://auth.lomi.dev`
and client ID `lomi-desktop`. Release builds always use these production values.
For local development only, compile-time `LOMI_AUTH_ORIGIN` and
`LOMI_AUTH_CLIENT_ID` overrides are honored by debug builds. The local portal
runs at `http://localhost:4321` and proxies API requests to the auth API on
`http://127.0.0.1:3001`; start both local services before launching Lomi with:

```sh
LOMI_AUTH_ORIGIN=http://localhost:4321 \
LOMI_AUTH_CLIENT_ID=lomi-desktop-dev \
pnpm tauri dev
```

Origins and client IDs are validated by native code. Local HTTP is allowed only
for the debug build and only on the loopback address and port above. The renderer
cannot supply a server address, arbitrary browser URL, or Authorization header.

## Boundaries and persistence

`src-tauri/src/auth` owns HTTP, the loopback callback listener, credentials, and
the state machine.
`src/auth` renders a sanitized snapshot. IPC and `auth-state-changed` events never
include the session token, authorization URL, verifier, state, or callback code.
An attempt snapshot contains only its ID and expiry. Each snapshot has a monotonically
increasing revision so a delayed reply cannot restore an obsolete account state.
The existing global trusted-webview guard remains in place. Main and Settings may
read account state, start sign-in, reopen the current browser request, sign out,
and open the account portal. Cancelling a sign-in attempt remains restricted to
Settings. Browser children are not authorized app views.

Persistent sessions use a separate OS keyring service namespace and private
metadata below `account-auth/<storage-namespace>` in the application data directory.
Release builds retain the environment namespace, such as `auth-lomi-dev`.
Debug builds append `-debug`, using `auth-lomi-dev-debug` for production or
`development-debug` for the local portal. Debug and release builds therefore have
separate account sessions and can own their account storage concurrently. Sign in
separately in each build; existing sessions are not copied or migrated. This
storage suffix does not change the account service or Remote environment.
Metadata contains credential references and a cleanup journal, never the token.
A process lock protects ownership. A durable logout tombstone is written before
keyring deletion. A failed deletion remains journaled and cannot silently restore
the signed-out account. Unreadable metadata is preserved for recovery.

On macOS, Lomi disables Keychain password and access dialogs for its process.
Credentials that require interaction remain unavailable, and their references and
pending cleanup are preserved. Settings → Account explains the recovery choices:
use Check connection to retry access, explicitly Sign out of this device before
replacing the stored session with a persistent sign-in, or turn off Remember me on
this device to select a session for this process. Damaged metadata or storage
owned by another Lomi process must be resolved first; these choices do not bypass
native storage ownership and recovery checks.
Lomi never automatically falls back to memory when persistent storage fails.
If remote revocation cannot be confirmed, the UI warns that the server session
may remain active. It reports successful local sign-out only after the logout
tombstone was saved, even when key store cleanup must be retried.

The Remember me switch explicitly selects persistent storage or memory for the
current process. There is no plaintext fallback. Replacing an unresolved persistent
reference requires a durable tombstone; unreadable or unowned metadata blocks
replacement. The account keyring service is separate from Chat AI credentials.

Closing Settings or navigating to another settings page does not cancel sign-in.
Cancel, sign-out, a new attempt, and application exit invalidate native work.
Late responses cannot publish an old attempt. HTTP has bounded timeouts and
response sizes, and does not follow redirects. The loopback listener accepts only
the exact callback route with one valid code and matching state, and closes on
success, cancellation, or expiry. Code exchange is attempted once because the
browser code may be single-use.

Offline is an unknown remote state, not proof of an authenticated online session.
Local sign-out is available without the server; the UI reports when remote
revocation was not confirmed. Server account status and session validity remain
the authority for online requests.

## Development and verification

The adjacent `auth-app` and `auth-frontend` repositories contain the API and
portal. Follow [the deployment guide](https://github.com/lomi-dev/auth-app/blob/main/DEPLOYMENT.md)
(requires access to the private API repository) and use Bun 1.4.2 there.
This desktop repository continues to use pnpm and Cargo.

```sh
pnpm check
node --experimental-strip-types --test tests/auth-state.test.ts
pnpm exec playwright test tests/ui/account-settings.spec.ts
cargo test --manifest-path src-tauri/Cargo.toml --locked auth::
cargo check --manifest-path src-tauri/Cargo.toml --locked
```

Native HTTP tests use isolated loopback servers. Opt-in macOS Keychain tests in
`src-tauri/src/auth/storage.rs` use randomized test service names and must be run
explicitly with `--ignored` and their exact test names. They never use an existing
account entry. Browser UI tests mock Tauri IPC and do not qualify native windows.

The implementation was locally checked on macOS ARM64. Real GitHub OAuth,
production TLS/DNS, release packaging, and Windows/Linux credential stores need
separate qualification before public release. The full evidence and remaining
release conditions are recorded in documents 14 and 18 of the adjacent local
`auth-plan` directory, which is not published to GitHub.
