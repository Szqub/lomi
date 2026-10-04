# Test workspace Remote locally

Remote remains enabled while any desktop terminal produces data or receives
input, or the user interacts with a Lomi window (including browser panels).
Network heartbeats, account checks and rendering do not reset the activity timer.
After more than one hour without activity across all windows and terminals,
Remote pauses and closes its channels. Local terminals and workspace consent
remain intact. **Resume remote** in the workspace footer or Remote settings
reconnects without restarting Lomi. Explicitly stopping workspace sharing still
revokes that workspace's access, including while paused.

Active native approvals renew with identical scope before their deadline. Mobile
refresh and browser identity checks renew bounded account sessions. Deploy the
matching desktop, mobile, standalone `remote-web` and `auth-app` changes together;
the database owner must apply `auth-app/ops/remote-database-permissions.sql` so the
private exchange role can update validity deadlines without reading source tokens.

The terminal helper restores its checkpoint and ordered journal after failure.
If an individual terminal exceeds the bounded recovery budget, its remote model
becomes unavailable rather than publishing incomplete terminal state. Local
terminal output continues, and other terminal models recover independently.

The local demo runs on macOS ARM64 with a genuine isolated test account, retained
native terminals, PostgreSQL, the account API and the browser's release WASM
crypto. It supplies test login sessions; interactive GitHub OAuth is a separate
check.

Desktop release 0.5.3 is published, with native live hosting enabled only on
macOS ARM64. `LIVE_PRODUCTION_QUALIFIED` is true; the general
`PRODUCTION_QUALIFIED` and mailbox `MAILBOX_PRODUCTION_QUALIFIED` flags remain
false. The local 0.5.2 native/browser integration run passed 15 checks. Release
0.5.3 passed its 36 focused native authentication tests and 12 checks each
against the public staging and production services, including normal desktop
PKCE and browser RemoteLogin approval, encrypted terminal input, observation
after local takeover, unsharing, revocation, background operation and Quit.
The identity provider's interactive GitHub OAuth flow is a separate check.

Production is available at https://remote.lomi.dev. Use desktop 0.5.3 or a later
compatible release; native Remote requests declare support for safe atomic
control renewal. A compatibility declaration supplements session authentication
and the host's signed grant scope.

```sh
cd /Users/woro/Documents/Lomi/lomi-remote-live
pnpm remote:demo
```

Wait for `remote_manual_demo` with `ready: true`. The opened browser and desktop
use the same account. A separate browser must log in before discovering Remote.

1. In the desktop workspace list, right-click **Remote test workspace** and choose
   **Share remotely**.
2. In the browser, open that workspace. Its three terminal panes appear together,
   including the two panes in the inactive **Background terminals** tab. Selecting
   a terminal establishes an encrypted connection and requests control automatically.
3. Run `pwd` or `echo hello`. Create another terminal or split in the desktop;
   it joins the shared workspace automatically.
4. Refresh the browser. It enrolls a fresh ephemeral browser identity, discovers
   the workspace and reconnects with a fresh terminal snapshot.
5. Close the desktop window and continue in the browser. Reopen Lomi from the Dock.
   Local typing takes control back; activating the browser terminal requests it again.
6. Right-click the workspace and choose **Stop sharing**. Browser access closes;
   local terminals continue running.

No fingerprint, pairing token or per-terminal configuration is needed. Workspace
sharing permits authenticated browsers of the same account. Account enrollment
is trusted to establish initial peer ownership; the native host signs the exact
workspace and terminal scope, and transport remains encrypted end to end. Actual
workspace names and terminal titles travel inside the encrypted connection.

This increment independently models at most 32 local terminal processes across
the desktop, including unshared terminals. A workspace containing a terminal
outside that budget reports an explicit error; it is never partially shared.
Local terminals continue working. The shared workspace and terminal limits are
also 32. Increasing the observer budget or introducing on-demand state bootstrap
requires separate terminal-state qualification.

Established channels reconnect in observing mode with fresh keys and snapshots.
Activate a terminal to request control after a connection or workspace scope
change; automatic renewal never restores a lease preempted by local typing. Unconfirmed input
is never replayed. An explicitly revoked browser enrollment does not silently
regain access during the same page lifetime.

Closing the demo browser leaves the desktop running. Press `Ctrl+C` in the
launching terminal to stop the demo. Explicit desktop Quit stops its sessions
through the normal application close guards.

Each run uses isolated app data and a private directory under
`/tmp/lomi-remote-live-e2e/native-run-*`; it does not load normal Lomi workspaces.
The launcher removes copied fixture credentials and app data only after its owned
native process group stops. Private diagnostics and receipts remain in that
run directory.

The local services are prepared separately:

```sh
cd /Users/woro/Documents/Lomi/auth-app-remote-live
bun test/integration/remote-live-services.ts serve
```

If the fixture has expired, renew it before launching:

```sh
bun test/integration/remote-live-services.ts renew
```

After logging out, mint a fresh isolated test browser login and reopen only the
browser; existing desktop sessions can stay running:

```sh
bun test/integration/remote-live-services.ts browser-login
cd /Users/woro/Documents/Lomi/lomi-remote-live
pnpm remote:browser:demo
```

These helpers require the existing private fixture files and isolated database;
none connect to production. Identity uses `127.0.0.1:4324`, browser/API `4322`,
relay `3004`, and manual desktop development server `1448`.

```sh
cd /Users/woro/Documents/Lomi/lomi-remote-live
pnpm test:remote:native
```

The automated native/browser check uses desktop port `1449`. Its receipt records
only checks actually completed. The lifecycle regression advances the activity
clock past one hour through a debug-only probe; it does not wait for a physical
hour. Two pause/resume cycles must close the existing relay sockets, retain
workspace consent and native terminal identities, and accept encrypted browser
input through the public Resume command in the same desktop process. Local
input while paused must continue working without automatically resuming Remote.

The helper recovery check kills the actual terminal helper child. It compares
the restored snapshot and sequence exactly, checks that terminal epochs stay
unchanged, and requires encrypted input over a replacement connection. These
probes require `remote-probe` and isolated fixture app data; release builds reject
qualification features.

The current macOS ARM64 lifecycle regression passed all 17 checks, including
both accelerated idle/resume cycles and recovery after killing the real helper. Release 0.5.3 also passed the
public staging and production runs, with 12 checks each. Signed release artifact
verification and production backup restoration are recorded separately in the
workspace rollout evidence.
