# Native router support for the eight requested CLIs

Source implementation and offline validation updated 2026-10-04. Implementation
and source review finished before the test gate reopened. Real accounts remain unqualified. This is
the current scope for Claude Code, Codex, Grok,
Kimi, Kilo, OpenCode, Pi and Antigravity. It supplements the broader
[qualification matrix](cli-router-adapter-qualification.md).

There are two account paths. API Gateway uses reviewed HTTPS destinations and
keys stored by Lomi; the native client receives only an ephemeral loopback token.
Native account terminals let the unmodified CLI perform subscription/provider
login, renewal and coding in a persistent private profile. Native account
credentials are not converted into generic Gateway API keys.

| CLI                 | Reviewed native versions             | API Gateway protocol | Native account login                                                                             | Saved Gateway history                                                                     |
| ------------------- | ------------------------------------ | -------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| Claude Code         | 2.1.63, 2.1.287                      | Anthropic Messages   | Private `CLAUDE_CONFIG_DIR` and HOME; native login                                               | Existing workspace conversation ID, excluding subagents/sidechains                        |
| Codex               | 0.160.0                              | OpenAI Responses     | Private `CODEX_HOME`; new accounts use file credentials; existing keyring configuration retained | First rollout metadata ID and workspace; explicit `resume ID`                             |
| Grok                | Lomi source build 1.0.45             | OpenAI Responses     | Private `GROK_HOME`; native browser/account login                                                | Strict native `--resume`; missing workspace history fails                                 |
| Kimi                | npm 2.1.1; Python 1.52.0 for Gateway | OpenAI Chat          | npm `KIMI_CODE_HOME` with native file OAuth; Python subscription fallback not admitted           | npm existing index/state/wire and explicit session ID; Python strict native continue      |
| Kilo                | 7.8.3                                | OpenAI Chat          | Private HOME/XDG; native provider login                                                          | Existing top-level workspace DB session with message history                              |
| OpenCode            | 1.18.33, 1.18.34                     | OpenAI Chat          | Private HOME/XDG; native provider login                                                          | Existing top-level workspace DB session with message history                              |
| Pi                  | Mario 0.73.1, Earendil 1.0.1         | OpenAI Responses     | Private `PI_CODING_AGENT_DIR`; native `/login` selects provider                                  | Existing workspace JSONL with messages; explicit session file                             |
| Antigravity (`agy`) | 1.2.16                               | Gemini               | Exact official macOS ARM64 artifact; private HOME and native remote OAuth/file-storage mode      | Existing conversation map, valid SQLite metadata/protobuf/steps; explicit conversation ID |

Every listed Gateway keeps native tools, permissions, compaction and history.
Codex uses `--no-daemon` and Kilo `KILO_NO_DAEMON=1`; existing external daemons
cannot own these terminals. Kimi emitters are family-specific: npm uses `openai`,
`KIMI_CODE_HOME` and forced secondary model; Python uses `openai_legacy`,
`KIMI_SHARE_DIR` and `--config-file`. Pi passes its local API token explicitly so
stored native provider credentials cannot override the Gateway provider.
Antigravity API mode selects `modelProvider=gemini` and uses
`GEMINI_API_KEY`/`GOOGLE_GEMINI_BASE_URL`, with native Gemini discovery and token
counting routes. Machine/project provider overrides are refused before dispatch.
OpenCode/Kilo automatic self-updates are disabled for version inspection,
Gateway and native account terminals, retaining the reviewed executable version.

## Current implementation scope and remaining gates

All eight clients have native API Gateway profiles using their same wire family,
and all eight have persistent private native account terminals. Supervised native
turns are implemented for Claude, Codex, Grok, Kimi, Kilo, OpenCode and Pi.
Antigravity supervised turns remain disabled: the reviewed hook contract can fail
open and its control protocol does not establish the required pre-effect
approval fence. Interactive
Antigravity account terminals and same-family API Gateway are separate paths.

Kimi npm 2.1.1, Kilo 7.8.3 and Earendil Pi 1.0.1 are installed in
`~/.local/share/lomi-router-clis`, with launchers linked from `~/.local/bin` by
the pinned installer. No shell startup files were changed. Isolated installed-client
version probes passed with network and shared credential reads denied. The installer now records the complete shipped
runtime file closure, including Pi's chunks and dependencies; legacy receipts
upgrade only after comparison with a fresh integrity-verified extraction.

Antigravity's owned native account profile sets `agentMode=default`, launches
with `--mode=default`, and adds `write_file(*)`, `command(*)`, `unsandboxed(*)`,
`mcp(*)`, `read_url(*)` and `execute_url(*)` to `permissions.ask`. Existing Deny
rules and unrelated settings are preserved. Private parents, atomic private
settings writes and project/machine policy admission protect this profile.
These are native PTY prompts; they do not establish Lomi structured approvals.
The effective native policy merge still requires real-client qualification.

The converter implements the canonical text/function-tool subset across all
12 directed pairs of Anthropic Messages, OpenAI Chat Completions, OpenAI Responses
and Gemini. Same-family traffic retains exact bytes. Cross-family conversion
preserves supported system prompts, call IDs/results, sampling, inline images,
finish reasons and reported usage; buffered SSE must have a validated terminal
native event before converted output is released. Unknown usage remains unknown.
Opaque account references, encrypted reasoning, signatures, hosted/custom grammar
tools and unrepresentable options fail before dispatch instead of losing context.
Cross-family token counting is unsupported and never estimates or generates.

Actual Codex 0.160.0 defaults require encrypted reasoning output and all-turns
context, so native Codex Gateway profiles accept Responses destinations only.
The public Chat/Responses subset can map documented reasoning effort, text format,
verbosity, metadata and cache keys; this does not qualify Codex native payloads
for Chat. Other clients can choose four protocols but still undergo per-request
feature validation. Anthropic cache breakpoints and user metadata, for example,
are not silently removed to make a cross-family request pass.

Native supervised turns do not automatically retry an empty result or switch
accounts: unreported plugin/hook startup effects cannot prove an effect-free
attempt. API Gateway retains its separate pre-response HTTP 429 failover contract.
Ordinary settled supervised checkpoints resume the exact session in the original
account. The qualified Pi transfer described below provides a separate reviewed
cross-profile path. Uncertainty offers **Review in native account terminal**, which opens
that exact session without replaying the task and retires the supervised
checkpoint. Recorded-completion acknowledgement never launches a CLI or turn.

Native account refresh is explicit and read-only. Source-qualified native auth
and identity observations can update a matching unchanged profile; raw quota
windows remain metadata with unknown model scope. Unqualified observations have
router quota status **Unknown** and cannot enable percentage balancing. Claude
subscription quota remains unknown; its statusline parser is a fixture candidate,
not a production collector. Pi and
OpenCode have no universal verified native identity/quota contract. Pi identity
and quota qualification remain a blocker, rather than fabricated authentication.
Agy's source-qualified collector uses only the standalone
`-p /usage --output-format json` operation, without an agent turn. It preserves
native bucket IDs, windows, amounts and resets; it does not establish account
identity, model scope or comparable balances. No generic OAuth proxy or paid
quota prompt is used. The new collector fixtures passed offline.
Successful parsing alone does not promote Antigravity to authenticated: freshness
of its backend authentication is unqualified, so the profile stays unverified.

## Account selection and continuation

Gateway needs at least two reviewed distinct credential/destination bindings.
Only a complete HTTP 429 rejection before downstream bytes can select another
account. Partial streams, transport failures, tool effects and ambiguous requests
are never replayed. Rate-limit cooldown is not a measurement of subscription
balance. Successful Responses/Gemini/custom-Anthropic inference retains creator
account affinity; auxiliary token counts do not create context or pin an account.
OpenAI Chat and the narrower official Anthropic pool retain their existing safe
pre-response rotation rules after resume.

Stop waits for the native process group, PTY tail and HTTP journal to drain. Main
then offers an explicit **Resume CLI** action. Restore/restart never launches a
CLI or submits a task. Resume checks the exact CLI family/version, workspace,
model, clean journal and creator credential revision/destination. It rotates only
owned routing configuration and the local token, preserving native auth/history.
Separate immutable journal generations retain prior receipts. Missing,
changed, corrupt, foreign-workspace or empty history cannot silently become a
fresh conversation. Legacy Gateway runs without a continuation record are
preserved and rejected for automatic resume. Legacy Antigravity `.pb` history is
also preserved; this adapter qualifies current SQLite history only.

Subscription profiles are individually selectable through their native account
terminals. Pi has provider subscriptions; Kilo/OpenCode plan credentials and
external provider OAuth are separate native choices. Codex retains its existing
authenticated subscription reader and structural text-exhaustion routing; explicit
native refresh observations do not establish comparable model-scoped quotas.
There is no universal eight-client automatic subscription failover or native
history transfer. Earendil Pi 1.0.1 has a bounded private cross-profile history
transfer for `openai`/`openai-completions` only. It preserves the admitted native
history losslessly and rejects opaque signatures, deferred or remote content,
unknown extensions and unresolved tool calls. Main previews the transfer and
requires explicit Apply; the immutable intent and receipt bind the reviewed
revision, grants and actual history-file SHA. The original checkpoint remains
unchanged until a successful destination checkpoint is created. Apply dispatches
no turn; Send or Continue is a separate action. Offline fixtures passed; real-account transfer remains unqualified.
An unlaunched copy stays bound to the reviewed destination revision. Changing
that account or target does not silently reuse or overwrite the prepared copy;
the original source history remains available. There is no automatic abandonment
of such artifacts. Resume the original account to obtain another settled
checkpoint before preparing a different transfer.
Claude subscription login stays in
the native client, consistent with its
[documented native-client authentication contract](https://code.claude.com/docs/en/legal-and-compliance).

## Source comparison

[Claude Code Router 3.1.1](https://github.com/musistudio/claude-code-router/tree/471e715c20cfa855c681f6d31dc652164d4fa654)
provides seven relevant profile writers and protocol adapters. Lomi follows the
verified endpoint/configuration concepts while retaining its own private homes,
leases, frozen account grants and durable receipts. It does not copy global
profile imports, shell command interpolation or symlink-based credential sharing.

[FreeRouter](https://github.com/r2hu1/freerouter/tree/cab153114d1e392a81a5b55270d7e28e420dd3c4)
provides Chat routing and provider health selection. It does not itself supply
native Anthropic/Responses/Gemini clients or eight CLI profile integrations.
Buffered pre-response failover is useful; Lomi does not treat midstream errors
as successful stream completion.

[CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI/tree/8ef43e4df3b216a42493105d31c2873b69191473)
adds specialized OAuth executors and protocol translation. Its Antigravity
provider is a CloudCode upstream, separate from native `agy` CLI integration.
Lomi now implements bounded conversion of the canonical public text/function-tool
subset across the four wire protocols. Native Codex remains Responses-only, and
unsupported native features fail before dispatch. Native tools/attachments still
require provider support for the admitted wire protocol and feature set.

Primary CLI references: [Antigravity installation/auth](https://www.antigravity.google/docs/cli/install/),
[headless operation](https://www.antigravity.google/docs/cli/headless/),
[Pi providers](https://pi.dev/docs/latest/providers),
[Kimi npm artifact](https://registry.npmjs.org/@moonshot-ai/kimi-code/-/kimi-code-2.1.1.tgz),
[Kilo source](https://github.com/Kilo-Org/kilocode/tree/59f1428abb5fe782ee7bd4d258e72a08b74aadb4),
[OpenCode 1.18.33 source](https://github.com/anomalyco/opencode/tree/51ef4be1d3c122f18fefb510dca8d778571f4f18),
[1.18.34 source](https://github.com/anomalyco/opencode/tree/aec0b9a6d8898f68f923aaf08b7306d931fd9d76),
[Codex CLI source](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/cli/src/main.rs).

## Grok artifact and build

Public source commit `2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8`, source revision
`559751fdcec02d413e4c57c8832ab275e4f44980`, is built with
`cargo build --locked -p xai-grok-pager-bin --release`. macOS ARM64 artifact SHA256:
`b985241c2dc8ff135994174ad65844278e470afd1b171ca6737b4960bdb66de4`.
Lomi checks compiled manifest, exact source-stamped version, ownership and binary
identity before version inspection and launch. PATH Grok is never substituted.

Use `node scripts/build-router-grok.mjs` to prepare the resource on a matching
build platform. Changed/rebuilt bytes are staged as a candidate instead of
automatically granting trust. The resource lives under `src-tauri/resources/grok`
and is mapped to `router-grok` in the bundle. Third-party license notices are
included. Signing must preserve the reviewed resource bytes or undergo explicit
artifact requalification. Other Grok platforms require their own reviewed build.

## Validation

Implementation and source review of the installer, Antigravity policy/usage and
Pi transfer finished before tests started. Current local validation passed:

- `pnpm check`, `pnpm build`, and 250 host plus 35 AI-runtime tests.
- 32 mocked Playwright router/editor cases, including two new Pi transfer cases
  covering explicit consent, changed revisions, nested cancellation, retained
  drafts and zero task dispatch during handoff.
- Full locked Rust workspace: 1038 passed, 0 failed, 28 ignored. The focused
  67 native tests are included in this total.
- Installer migration to complete runtime receipts and subsequent idempotent
  reuse passed. Kimi `2.1.1`, Kilo `7.8.3` and Pi `1.0.1` version probes passed
  in disposable private homes with network/global writes and shared credential
  reads denied.

Strict production Clippy and the updated desktop artifact are recorded in the
[central validation record](cli-router.md#validation-record).
No configured profiles, real login or live paid qualification are available.

The following results are historical, from the previous supervised-native,
native-refresh and cross-protocol phase. Its implementation and source review
finished before tests began. That macOS ARM64 validation passed:

- `pnpm test`: 250 host tests and 35 AI-runtime tests; no failures.
- `pnpm check` and `pnpm build`: passed, including SDK contract verification.
- Mocked Playwright coverage: 40 distinct cases across router, editor/coding
  effects, application-close and agent-terminal-profile suites. The native
  recovery bridge carries the original account and run revision without sending
  a prompt; editor input remains unchanged across approval polling. The rendered
  native editor interaction screenshot was inspected.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked`:
  1022 passed, 0 failed, 28 ignored. Ignored cases are not passes. Coverage includes
  all 12 directed protocol pairs, native permission/session lifecycles, shared
  account deduplication, canonical private checkpoint history, changed history,
  replaced workspace directories and checkpoint retirement before recovery.
- Strict production Clippy (`-D warnings`), Rust formatting, targeted Prettier
  checks and `git diff --check`: passed, without blanket lint suppression.
- `pnpm tauri build --no-bundle`: passed; that phase's optimized macOS ARM64
  executable and local resources are available in `src-tauri/target/release`.
- Repository-wide `pnpm format:check` still reports three unchanged files under
  `packages/remote-terminal-runtime`: `src/index.mjs`, `src/model.mjs` and
  `tests/ipc.test.mjs`. They are outside the router changes.

The historical optimized desktop build is recorded in the
[central validation record](cli-router.md#validation-record). No live account is
available for qualification; no successful native subscription turn, fresh
comparable quota or cross-account native continuation is claimed. Remaining gates
include Antigravity's fail-open hooks, Pi identity/quota, installed-client
admission and real-account cancellation/approval/recovery trials. No authentication
or provider calls ran in this phase. This is not complete or 100% qualification.

The following results describe the earlier same-family Gateway/account-terminal
phase only; they do not validate the new code. Historical validation ran on macOS
ARM64; Linux, Windows and WSL are not qualified.

- `pnpm test`: 248 host tests and 35 AI-runtime tests passed.
- `pnpm check` and `pnpm build`: passed, including SDK contract verification.
- Router, editor/coding effects and application-close Playwright suites: all
  33 cases passed with mocked native commands. Rendered screenshots inspected.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked`:
  946 passed, 0 failed, 28 ignored. Ignored cases are not counted as passes.
- Rust coverage includes native profile isolation, exact version admission,
  retained auth/history during routing configuration changes, parent/child
  session selection, SQLite/WAL snapshots, Kimi activity/deletion handling,
  Antigravity metadata framing, updater restrictions, HTTP token counts and
  context affinity. Local PTY fixtures cover process and journal drain.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings`
  and Rust formatting: passed. Final source review has no remaining material
  findings.
- `pnpm format:check` and `git diff --check`: passed.
- `pnpm tauri build --no-bundle`: passed; produced the optimized macOS ARM64
  executable at `src-tauri/target/release/lomi`. No installer was published.

Isolated native version probes used disposable private HOME/XDG paths, denied
network/global writes and blocked reads of shared credential files. Codex
`0.160.0`, Claude `2.1.287`, OpenCode `1.18.33`, owned Grok
`1.0.45 (2bdd1d6a6369)` and official Antigravity `1.2.16` matched reviewed
admissions. Grok and Antigravity artifact hashes also matched. Kimi, Kilo and Pi
were absent from this host's PATH at that time. They have since been installed in
the owned local tree described above; their new isolated version probes passed.

The cached Earendil Pi npm entry also reported `1.0.1` successfully with a
byte-identical Node `22.22.3` copied into the disposable runtime directory.
An initial probe crashed in Node/libuv/CoreFoundation's process-title setter
under runtime ancestry read denial; a minimal control reproduced it. The control
and Pi both passed from the private runtime path without relaxing network,
credential-read or global-write restrictions. At that historical stage, Kimi's
cached package lacked external dependencies and Kilo's cache lacked its platform
binary. The current installed Kimi, Kilo and Pi version probes passed.

No real login, private credential read or paid provider call was performed.
These checks establish local implementation behavior, not successful live
provider/account execution or seamless cross-account continuation for all eight.
