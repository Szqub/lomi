# CLI router implementation and qualification

Current eight-client scope: [native API routing, account login and guarded history resume](cli-router-focus-eight.md).

Implementation date: 2026-10-04. This partially implements `../router-plan`,
expanded to the complete Lomi CLI catalog. Full qualification and automatic account failover across
all 31 CLIs remain incomplete. Managed text, mediated Codex coding and native
gateway terminals have separate admission and continuation contracts.

## Current behavior

The following table describes managed text mode.

Settings → Agent control → Router lists all 31 recognized CLIs with independent
capabilities. Agents → Open router selects an enabled pool and shows
workspace-scoped history. A catalog entry does not enable a managed adapter.
Managed text model catalogs are provided by the native adapter. Starting requires an
explicit model choice; Codex additionally shows only that model's supported
reasoning efforts. Changing CLI clears the previous model and effort. Clients
without a fixed catalog retain explicit model input.

| CLI             | Account binding                                                                                                           | Router execution                                                                                                                   | Quota                                              | Continuation                                                                                           |
| --------------- | ------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Codex           | Dedicated `CODEX_HOME`; client-owned login and authenticated account/principal binding                                    | Pinned `0.160.0` app-server text turns with owned tool-free model catalog and explicit effort; new profile terminals               | Native subscription windows and optional balancing | Logical text failover only for a terminal `usageLimitExceeded`, empty output and confirmed child drain |
| Claude Code     | Dedicated `CLAUDE_CONFIG_DIR`; immutable keyring API bindings                                                             | API text turns, pinned to `2.1.287`, tools disabled                                                                                | No qualified reader or invented API percentage     | Fresh conversation for every turn; explicit recovery                                                   |
| Gemini CLI      | Dedicated `GEMINI_CLI_HOME`; immutable keyring API bindings                                                               | API text turns, pinned to `0.42.0`, tools disabled                                                                                 | No qualified reader or invented API percentage     | Fresh conversation for every turn; OAuth verification disabled                                         |
| Pi Coding Agent | Fresh private PI_CODING_AGENT_DIR; immutable OpenAI API binding                                                           | API text turns pinned to 1.0.1; exact source-catalog model allowlist                                                               | Unavailable                                        | Fresh conversation only; explicit recovery                                                             |
| Goose           | Fresh private GOOSE_PATH_ROOT/HOME and immutable OpenAI API binding; shared system config refused                         | API text turns pinned to 1.53.0; chat mode, no profile/extensions                                                                  | Unavailable                                        | Fresh hidden CLI session; explicit recovery                                                            |
| OpenClaw        | Fresh private home/state/config and immutable OpenAI API binding                                                          | Text turns pinned to `2026.9.8`, wildcard tool denial and one JSON result                                                          | Unavailable                                        | Logical text history; explicit recovery                                                                |
| Mistral Vibe    | Fresh private home/config and immutable OpenAI API binding; Mistral credentials absent                                    | Legacy public app-server pinned to `2.25.8`; correlated final session/runtime reads                                                | Unavailable                                        | Logical text history; explicit recovery                                                                |
| OpenCode        | Fresh HOME/XDG/database roots and immutable OpenAI API binding; shared managed preferences and ancestor Git state refused | API text turns pinned to `1.18.34`; plugins, tools, MCP and project discovery disabled                                             | Unavailable                                        | Logical text history; explicit recovery                                                                |
| Qwen Code       | Fresh private QWEN_HOME and immutable OpenAI API binding; owned HTTP/SQLite session store                                 | Text turns pinned to `0.24.7`; private Hosted Harness HTTP/SSE protocol, disabled tools and startup discovery                      | Unavailable                                        | Logical text history; explicit recovery                                                                |
| Kimi Code       | Fresh KIMI_CODE_HOME/config and immutable international Moonshot Open Platform API binding                                | Native `2.1.1` REST/WebSocket turns; `kimi-k2.6`, thinking disabled, authoritative completion and canonical messages               | Unavailable                                        | Logical text history; explicit recovery                                                                |
| Hermes Agent    | Fresh HERMES_HOME/HOME; immutable native Anthropic API binding and pinned installed source inventory                      | Text turns pinned to stable v2026.9.24 / 0.21.5; claude-sonnet-4-6, disabled tools/discovery and correlated stream-json completion | Unavailable                                        | Logical text history; explicit recovery                                                                |
| Crush           | Fresh private HOME/config/state and native Anthropic key; macOS only                                                      | Native 0.97.1 Unix-socket REST/SSE text turns, builtin tools and startup effects fenced                                            | Unavailable                                        | Logical text history; explicit recovery                                                                |
| Remaining 19    | Managed text auth/execution unavailable or unqualified                                                                    | Managed text unavailable; separate gateway candidates below                                                                        | Unavailable                                        | Unavailable                                                                                            |

Remaining entries: Antigravity CLI, Cursor CLI, GitHub Copilot CLI,
Aider, Cline CLI, Kilo Code CLI,
Kiro CLI, Factory Droid, OpenHands CLI, Continue CLI, Amp, Auggie,
Open Interpreter, Grok Build, Junie CLI, Deep
Agents Code, Freebuff CLI, Trae Agent, and SWE-agent.

These counts describe managed text adapters: twelve on macOS, with nineteen
catalog entries outside managed text mode. Gateway candidates are listed separately.

## Native gateway coding terminals

Gateway mode is separate from managed text mode and mediated Codex coding.
The current macOS ARM64 build enables twenty source-pinned Gateway families,
including source-built Grok and native Antigravity. Other platforms require their
own artifact admission. A writer/version pin does not establish live account or
cross-account acceptance. See the [eight-client scope](cli-router-focus-eight.md).

| CLI family               | Native API protocol | Native version required          | Source                                                                                                                                                                       |
| ------------------------ | ------------------- | -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code              | Anthropic Messages  | 2.1.63 / 2.1.287                 | [Pinned source/artifact](https://registry.npmjs.org/@anthropic-ai/claude-code/-/claude-code-2.1.63.tgz)                                                                      |
| Codex                    | OpenAI Responses    | 0.160.0                          | [Pinned source/artifact](https://github.com/musistudio/claude-code-router/blob/471e715c20cfa855c681f6d31dc652164d4fa654/packages/core/src/profiles/service.ts)               |
| Grok Build               | OpenAI Responses    | 1.0.45 (owned macOS ARM64 build) | [Pinned source/artifact](https://github.com/musistudio/claude-code-router/blob/471e715c20cfa855c681f6d31dc652164d4fa654/packages/core/src/profiles/service.ts)               |
| Antigravity (`agy`)      | Gemini              | 1.2.16                           | [Native API mode](https://www.antigravity.google/docs/cli/install/)                                                                                                          |
| Kimi                     | OpenAI Chat         | npm 2.1.1 / Python 1.52.0        | [Pinned source/artifact](https://pypi.org/project/kimi-cli/1.52.0/#files)                                                                                                    |
| Pi                       | OpenAI Responses    | 0.73.1 / 1.0.1                   | [Pinned source/artifact](https://registry.npmjs.org/@mariozechner/pi-coding-agent/-/pi-coding-agent-0.73.1.tgz)                                                              |
| OpenCode                 | OpenAI Chat         | 1.18.33 / 1.18.34                | [Pinned source/artifact](https://github.com/musistudio/claude-code-router/blob/471e715c20cfa855c681f6d31dc652164d4fa654/packages/core/src/agents/opencode/profile-config.ts) |
| Kilo                     | OpenAI Chat         | 7.8.3                            | [Pinned source/artifact](https://github.com/Kilo-Org/kilocode/blob/59f1428abb5fe782ee7bd4d258e72a08b74aadb4/packages/opencode/src/config/config.ts)                          |
| Cline                    | Anthropic Messages  | 3.0.68                           | [Pinned source/artifact](https://github.com/cline/cline/blob/241c1884a7461ef35f6c384a027a38e8d03b3b33/sdk/packages/core/src/services/llms/provider-settings.ts)              |
| Hermes                   | Anthropic Messages  | 0.21.5                           | [Pinned source/artifact](https://github.com/NousResearch/hermes-agent/blob/f97608f178d1ffeca59860195ab7da295f7c8e5f/agent/anthropic_adapter.py)                              |
| Mistral Vibe             | OpenAI Chat         | 2.25.8                           | [Pinned source/artifact](https://github.com/mistralai/mistral-vibe/blob/7c19608af06f6c61d63f8f7a5c3430da73fba2ab/vibe/core/config/vibe_schema.py)                            |
| Goose                    | OpenAI Chat         | 1.53.0                           | [Pinned source/artifact](https://github.com/block/goose/blob/v1.53.0/crates/goose/src/providers/openai.rs)                                                                   |
| Aider                    | OpenAI Chat         | 0.86.2                           | [Pinned source/artifact](https://pypi.org/project/aider-chat/0.86.2/#files)                                                                                                  |
| OpenHands                | OpenAI Chat         | 1.16.0                           | [Pinned source/artifact](https://pypi.org/project/openhands/1.16.0/#files)                                                                                                   |
| Open Interpreter         | OpenAI Chat         | 0.4.3 (Python)                   | [Pinned source/artifact](https://pypi.org/project/open-interpreter/0.4.3/#files)                                                                                             |
| Continue                 | OpenAI Chat         | 1.5.47                           | [Pinned source/artifact](https://github.com/continuedev/continue/blob/d3f60ba9dd3fb5bfd3c91d6fbb41ce1aa768db45/core/llm/llms/OpenAI.ts)                                      |
| Deep Agents Code         | Anthropic Messages  | 0.1.80 (deepagents-code)         | [Pinned source/artifact](https://pypi.org/project/deepagents-code/0.1.80/#files)                                                                                             |
| Qwen Code                | OpenAI Chat         | 0.24.7                           | [Pinned source](https://github.com/QwenLM/qwen-code/tree/b12edec1401a28fc53cd9e714d5928b285071fc8/packages/cli/src/config)                                                   |
| Crush (macOS only)       | Anthropic Messages  | 0.97.1                           | [Pinned source](https://github.com/charmbracelet/crush/tree/e0baf255be53f932042b377630e233d4c33c3b4b/internal/config)                                                        |
| OpenClaw (`tui --local`) | OpenAI Responses    | 2026.9.8                         | [Pinned embedded TUI](https://github.com/openclaw/openclaw/blob/fc23bc864e4553c2d215e479eeec47b67a0bf943/src/tui/embedded-backend.ts)                                        |

Each gateway account profile carries an explicit HTTPS API destination and
protocol. The native owner starts an authenticated loopback gateway and a private
TTY, clears the child environment, and emits a fresh private HOME/config containing
only a local ephemeral client token. Provider keys stay native-side. Native tools,
permission prompts, compaction and history remain in the same CLI process while
eligible HTTP requests switch accounts. This mode does not apply the mediated
four-file-tool policy or claim to sandbox native plugins, shells or external tools.

Changing a profile API endpoint disconnects its retained old key. The new
destination remains disconnected until an API key is explicitly saved for it;
endpoint changes do not silently reuse credentials.
Equivalent API base aliases with the same credential are counted once, including
an unversioned `/api` and `/api/v1` that reach the same native endpoint.

Account switching is limited to an explicit upstream HTTP 429 before a response
has been forwarded. Partial responses, ambiguous transport failures and tool
execution are never replayed by the gateway. After the first successful response,
Responses, Gemini and custom Anthropic destinations retain account affinity.
Native Anthropic pools at `api.anthropic.com` pass thinking blocks and signatures
through unchanged. Anthropic documents opaque signature preservation and platform
portability; extending that to different accounts at the same API origin is a
source-based inference, **not a real A/B result**. See
[extended thinking](https://platform.claude.com/docs/en/docs/build-with-claude/extended-thinking)
and [thinking encryption](https://platform.claude.com/docs/en/build-with-claude/thinking#thinking-encryption).

Same-protocol Gateway requests and responses retain exact bytes. Canonical
text/function-tool conversion is source-implemented for all 12 directed pairs
across Anthropic Messages, OpenAI Chat, OpenAI Responses and Gemini. Converted
SSE is buffered until a validated terminal native event; truncation and errors
fail closed. Unsupported encrypted reasoning, opaque references, signatures,
hosted/custom grammar tools and options are rejected before dispatch instead of
losing context. Cross-protocol token counting is unsupported, never estimated.
Actual Codex 0.160.0 requires Responses destinations because its mandatory native
reasoning/context defaults lack a documented cross-protocol equivalent. Other
focused clients expose four choices subject to per-request feature validation.
Explicit custom HTTPS destinations require support for the admitted features.
API prefixes are retained, including
`https://openrouter.ai/api/v1` for its documented
[Chat endpoint](https://openrouter.ai/docs/api_reference/overview).
For example, `/compatible-mode/v1` is joined with `/chat/completions`, while
an unversioned `/anthropic` prefix receives `/v1/messages`. A configured final
`/v1` or `/v1beta` supplies the API version; otherwise the native request version
is appended. Ambiguous path segments, credentials, query and fragment are refused.
Select the exact provider model identifier and a supported protocol destination;
Codex requires Responses. This routing support is not a real OpenRouter account qualification.
Model/files/attachments and auxiliary cloud endpoints require
per-client workflow qualification. Project dotenv/config overrides and fixed
managed policy are admitted before key access and launch, and again at the owner
fence. Python Kimi 1.52.0 and npm Kimi Code 2.1.1 are distinct client families;
Python Open Interpreter 0.4.3 and deployment-only `deepagents-cli` must likewise not
be substituted for their listed family.

Save/Stop retains native HOME and history only after the HTTP request journal and
PTY output tail fully drain with the process group. An incomplete drain requires
explicit Recovery. The eight focused clients offer explicit guarded native history
resume after clean Stop. Exact client/version/workspace/model, prior receipts,
creator credential revision/destination and existing session IDs must match.
Restore never submits tasks or creates a substitute conversation.

Qwen pins its main, fast, compaction and vision models to one Chat provider in
private highest-priority system settings. Project `.env`, `.qwen/.env` and
`.qwen/settings.json` overrides must be absent; its separate DashScope web-search
service is disabled. Crush pins large and small models to one Anthropic provider,
disables default providers/model discovery and owns workspace state. Project
`crushrc`/`.crushrc`/`crush.json`/`.crush.json` and `/etc/crush/crush.json` must be
absent; its secret-free native release-check request remains. OpenClaw uses
embedded `tui --local`, with the canonical project workspace pinned in owned
configuration. Replace mode suppresses implicit discovery; main, utility,
compaction and subagent routes share one allowed Responses model with empty
fallbacks. Project dotenv is blocked and optional provider/embedding plugins are
disabled. These gates retain native core tools and permission checks; they do not
qualify API-file, media or external-service workflows.

Gemini still has managed text support but no qualified gateway emitter. Its
runtime agent aliases (including CLI help and skill extraction) resolve in agent
scopes outside the core model override, while default fallback chains merge with
configuration. A main-model pin alone does not establish one selected wire model
for these helpers. See the pinned [agent registry](https://github.com/google-gemini/gemini-cli/blob/v0.42.0/packages/core/src/agents/registry.ts)
and [model configuration service](https://github.com/google-gemini/gemini-cli/blob/v0.42.0/packages/core/src/services/modelConfigService.ts).

Droid documents [BYOK custom model base URLs](https://docs.factory.com/model-independence/byok).
Junie documents [EAP custom proxies](https://junie.jetbrains.com/docs/custom-proxies.html)
and [LiteLLM URL/key flags](https://junie.jetbrains.com/docs/parameters.html).
Overrides therefore exist, but exact released backend correlation, startup/auth
fallbacks and auxiliary routing remain unqualified; neither has a gateway emitter.

The profile approach references released
[CCR 3.1.1 / 471e715c20cfa855c681f6d31dc652164d4fa654](https://github.com/musistudio/claude-code-router/tree/471e715c20cfa855c681f6d31dc652164d4fa654).
Lomi emits fresh private profiles rather than importing global settings or
symlinking user credentials and sessions. CCR support alone does not qualify a
native installed executable.

## Supervised native coding and manual recovery

Seven focused clients have source-implemented supervised native turns: Claude,
Codex, Grok, Kimi, Kilo, OpenCode and Pi. All eight focused clients, including
Antigravity, retain private native account terminals and same-family API Gateway.
Antigravity supervised turns stay disabled because its hooks can fail open before
an effect and its control protocol does not supply the required supervision
contract; a source pin or interactive terminal is not approval qualification.

The current implementation installs npm Kimi 2.1.1, Kilo 7.8.3 and Earendil Pi
1.0.1 in `~/.local/share/lomi-router-clis`, with pinned-installer links in
`~/.local/bin`. No shell startup files were changed. Isolated installed-client
version probes passed with network and shared credential reads denied. Complete shipped runtime hashes/modes cover chunks and transitive
dependencies; a legacy receipt upgrades only against a fresh integrity-verified
extraction that matches the existing installation.

Antigravity private native account settings add six Ask rules:
`write_file(*)`, `command(*)`, `unsandboxed(*)`, `mcp(*)`, `read_url(*)` and
`execute_url(*)`. Owned settings use `agentMode=default`, the launch uses
`--mode=default`, and Deny rules and unrelated settings survive. Private parents,
atomic 0600 settings writes and project/machine policy admission apply. These
retain native PTY prompts only; Antigravity `native_turns` remains disabled.

Native permissions are explicit decisions for the reviewed native tool request,
run and generation. Native CLI file/shell/plugin policies still apply. There is
**no automatic native empty-output retry or account switch**: unreported plugin
or hook startup effects are not qualified evidence of no effects. API Gateway's
explicit pre-response HTTP 429 failover is a separate existing contract.

Supervised checkpoint resume requires a settled exact native session in its
original account. Uncertain results/effects require the explicit **Review in
native account terminal** action, which opens that exact saved session in the
original account without resending the task. Opening external TUI recovery retires
the supervised checkpoint, so later supervised resume cannot replay stale state.
Acknowledging a recorded native completion returns the recorded input to idle;
it never starts a CLI or dispatches another turn. Restore never starts either path.

Earendil Pi 1.0.1 now has a private cross-profile transfer limited to native
`openai`/`openai-completions` history. The lossless admitted subset rejects opaque
signatures, deferred/remote content, unknown extensions and unresolved tool calls.
Main requires a preview and explicit Apply bound to the reviewed revision,
account grants and actual history-file SHA in an immutable intent and receipt.
The original checkpoint changes only after successful creation of a destination
checkpoint. Apply never dispatches a turn; Send or Continue remains separate.
This bounded path does not establish universal eight-client native history
transfer or automatic subscription failover and has not been validated yet.

Explicit native identity/quota refresh is read-only and source-qualified. Saved
reports are sanitized metadata, not reusable auth proofs. Missing or unqualified
model scope remains router quota **Unknown**, even when native windows or balances
are displayed. Antigravity's collector now uses strict standalone
`-p /usage --output-format json` without an agent turn, preserving native bucket
IDs, windows, amounts and resets. It establishes neither identity nor model
scope or comparable percentage balancing. No independent Claude fresh quota or
universal Pi/OpenCode identity/quota is claimed. Pi identity/quota and Agy's
fail-open hooks/control protocol remain qualification blockers. Current collector
fixtures passed offline. Usage parsing alone does not establish fresh backend
authentication, so Antigravity remains unverified; effective native permission
policy merging still requires real-client qualification.

## Mediated Codex coding

The source and Main UI integrate a persistent Codex 0.160.0 coding path with four
root-owned tools: `lomi_list`, `lomi_read`, `lomi_write`, `lomi_apply_patch`. There is
no native shell in this mode. Each write requires one-use Main approval for the
exact path, before/after hashes, arguments, owner and generation. Main freezes the
actual loaded editor documents through filesystem completion; native admission
compares their disk revisions with the approved before hash. Dirty, conflicted or
saving documents require resolution before approval.

Private thread history, canonical full-turn hashes, rollout metadata, read/fsync
barriers and effect-journal receipts fence resume and turn dispatch. Unfinished
deltas remain uncertain observations. Project write leases reject overlapping
coding owners and conflicting Lomi/native-MCP mutations; external editors, PTYs
and external Git commands do not honor this lease. Atomic file exchange/hash
reconciliation preserves evidence of races rather than claiming external edits
cannot race. Cross-account opaque reasoning and exact native checkpoint recovery
remain pending real A/B and failure-injection qualification.

The [complete qualification matrix](cli-router-adapter-qualification.md) records
the documented contracts and remaining gates for every entry.

A Codex pool requires two enabled, verified, distinct account/principal groups
learned through authenticated quota refresh. Duplicate namespaces share capacity.
Managed API pools require two distinct API key bindings; separate keys do not
prove independent accounts or billing budgets. Verify checks local key presence,
not provider validity, and performs no inference.

Managed turns use private HOME/config/working directories. The displayed project
folder groups history but does not grant access to files. Claude uses
`--safe-mode`, `--tools ""`, and `--permission-mode dontAsk`. Gemini uses private
system settings and empty dotenv sentinels, an empty built-in tool list, and
disabled hooks, MCP, extensions and skills. Unexpected tools, unknown events,
version mismatch or uncertain completion stop execution.

Pi uses fresh attempt state, explicit owned system prompts, disabled discovery and
SSE transport. Its pinned model allowlist rejects fuzzy model identifiers before
credential access or dispatch. Session/provider retries, compaction and cache
warming are disabled. Success requires the authoritative assistant message,
matching provider/model, agent_settled and a clean process exit. This adapter is
source based; real installations, dependency identity and accounts are untested.

Goose uses fresh state, a disabled shared keyring, chat mode and no profile
extensions. It refuses shared system configuration before startup and again at
dispatch. Every assistant chunk needs matching provider/model provenance;
completion additionally requires a terminal record, clean EOF, written input and
clean exit. Ordinary profile terminals are unavailable for this adapter.
The pinned client retains finite internal SDK/empty-response retries (up to four
ordinary provider turns) and may make one reactive compaction call. These are
client behavior within one logical attempt, not a Lomi account switch. Installed
client/account behavior remains untested.

Codex uses separate ephemeral app-server authentication, an owned pinned model
catalog and empty environments at both thread and turn start. Startup rejects
shared system configuration and forced macOS preferences. It validates native
configuration, requirements, model/effort and account-bound ordinary usage before
inference. No filesystem, shell, patch, MCP, hooks or extension tools are available.
Pinned npm wrappers resolve to their verified native executable before the version
gate and process launch. Text messages retain commentary and the final answer in
native order; canonical completion must agree with saved deltas. Refresh callbacks
require renewed authentication and never receive another account's token.

Only a correlated terminal `usageLimitExceeded` after clean EOF/process drain
can block its authenticated quota group. If that attempt returned no text or
effects, the owner can continue the saved input on another frozen approved
profile, at most 32 attempts per invocation. Pins, revoked grants and unavailable
capacity prevent automatic switching. Partial text, authentication, policy,
transient rate limiting or uncertain completion require explicit review instead.
This is logical text continuity in a new conversation; native cross-account
thread/thinking/tool-state qualification and full coding-task failover remain
pending. The separate mediated coding source is described above.

OpenClaw uses owned provider configuration, disabled plugins/hooks/bootstrap and
wildcard tool denial. Its complete JSON envelope must confirm model/provider,
text-only content and no tool/Code Mode effects. Native overflow and timeout
recovery can make auxiliary requests/compactions despite disabled proactive
compaction (up to three overflow and two timeout compaction attempts).

Vibe uses its public `vibe-app-server` sibling with the legacy harness and a
generic OpenAI provider. Private roots, disabled keyring and absent Mistral keys
prevent the authenticated remote managed-configuration fetch. It denies callbacks,
checks runtime tools/discovery and requires final correlated session/runtime
reads, server close, EOF and exit. Native generic retries have a 300-second budget.

OpenCode uses separate original HOME/XDG roots and an owned configuration. Pure
mode plus disabled default plugins suppresses both external and internal plugin
hooks. Wildcard permission denial removes all model tools before a request.
Owned dependency lockfile data prevents startup npm installation; macOS managed
preferences and ancestor Git state must be absent before probing and dispatch.
Success requires correlated authoritative text, step completion, bounded JSONL,
written stdin, EOF and process drain. Existing installations remain untested.

Qwen uses the pinned private Hosted Harness protocol of its public serve command.
The owned local HTTP store commits journal/resource bytes before acknowledging
leases, compare-and-swap updates and immutable digests. A matching final assistant
resource must establish exact prompt, model and successful terminal text. The
adapter then confirms quiescence and sealed session deletion, consumes the SSE
administrative tail to natural EOF, reaps the owned daemon and joins all readers
and the store. Context is limited to 60 KiB and inline resources to 64 KiB. An
admission receipt is never completion; flattened errors never trigger automatic
account rotation. This private protocol requires the exact `0.24.7` client.

Kimi uses the native `2.1.1` generation and its pinned REST/WebSocket protocol.
The only approved backend is international Moonshot Open Platform at its fixed
endpoint with `kimi-k2.6` and explicitly disabled thinking. Kimi Code subscription
keys, regional keys and OpenAI keys are not accepted substitutes. The owned
private server token authenticates readiness, API and WebSocket requests. The
adapter handles native JSON heartbeats, enforces contiguous event and UTF-16 text
offsets, and requires successful matching turn/prompt completion plus canonical
user/assistant messages, fixed configuration and idle session. Protocol/provider
failures retain partial output for recovery. All paths abort or settle, shut down
and reap the daemon, then join its diagnostic readers. Real clients and accounts
remain unqualified.

Hermes uses native Anthropic API credentials and exact `claude-sonnet-4-6`, pinned
to stable `v2026.9.24` / distribution `0.21.5`. Before version inspection or
credential access, admission matches the official installer shim, pinned checkout,
venv Python, Anthropic SDK 0.87.0 and compile-owned source inventories. An owned
isolated bootstrap bypasses shell wrappers, editable `.pth` loading and startup
customizers. Bytecode shadows, shared system site packages, installation
environment overrides and repair markers are refused.
A fresh owned config disables tools, plugins, MCP, hooks, auxiliary requests and
credential fallbacks. Environment safe mode retains this configuration; guest
onboarding and bundled/optional discovery are disabled. Seeded local model
metadata prevents startup model discovery. The public stdin query is prefixed
so path-shaped context cannot trigger native file preprocessing, and the full
input is bounded to 60 KiB. Completion requires correlated native model/session
events, authoritative final text, clean EOF and process/readers drain. OAuth,
quota and native resume are unavailable; real clients/accounts remain unqualified.

Grok's owned macOS ARM64 source build now enables native Gateway and subscription
account terminals. Its managed text driver remains unavailable. Public npm
semver is not substituted for the reviewed source/hash. Artifact admission is
compiled and cannot be replaced by settings, environment or a user-supplied hash.

All managed turns preserve explicit per-input/per-attempt text records. Completed
and uncertain partial responses remain distinct; legacy output is never assigned
invented message boundaries. The next prompt contains the bounded full logical
text history and refuses oversized context without truncation. Continue saved task
grants one new manual attempt with reviewed text. New pool accounts never receive
old runs' history automatically; credential rebinding revokes previous grants.
Review account access can explicitly grant or revoke access to an existing run's
saved messages and responses, including uncertain partial text. Consent binds the
reviewed run revision and every selected live account revision. A changed account
or history requires a new review. Active or draining attempts and closing views
reject access changes. Saving access neither starts nor resumes work; revoked
account pins are cleared.
Storage schema 5 distinguishes text, mediated coding, supervised native and
gateway execution modes.
Legacy schema 1/2/3/4 runs migrate without expanding account access or dispatching
work. Local coding, native and gateway source/UI validation has passed;
real-client/account qualification remains pending.

Output checkpoints use the expected per-turn byte offset: an uncertain commit
receipt can be retried without duplicating text, and conflicting or missing bytes
require recovery.

## Native contracts

`src-tauri/src/cli_router/` owns storage, credentials, selection and processes.
React receives sanitized views; it cannot supply executables, auth directories,
environment maps or credential identifiers. Settings manages profiles/keys/pools;
Main controls runs. Browser children cannot call these commands or receive
transcripts. Existing ordinary terminal features remain available.

Private SQLite storage uses WAL/FULL and a single native owner lock. Schema 5
request tombstones contain only ID, argument digest and committed revision.
Run creation binds the run ID to its canonical UUIDv4 request ID, so concurrent
creations cannot cause the UI to open another run. Replays return current state without executing callbacks or retaining transcript
copies. Schemas 1–4 migrate transactionally after validation; corrupt/future data is preserved.
The 100,000-entry request budget never expires IDs to permit duplicate effects.

API secrets have immutable revision-specific keyring entries. A durable journal
precedes mutation and restart reconciles unfinished cleanup. Pending removal
rejects verification, re-enable and rebinding. Profile removal cleans Lomi's API
secret but preserves CLI-owned login files. Run removal deletes visible Lomi
history; CLI files, external backups and filesystem retention are separate.

Input and dispatch intent commit before spawn. At most four native reservations
run concurrently; capacity rejection occurs before a start is persisted. Stop
cancels before launch and remains usable after persistence failure. Process-group
cleanup and leader reaping precede ownership release. After workers finish, a
successful recovery commit repairs a storage-failure latch without replay.

Quota collection is serialized and permits four concurrent bounded reads.
Publication fences profile revisions, principal/group identity, collection epoch
and block revision. Authenticated principal identity is retained when quota scope
is unsupported, so ordered new-terminal selection remains available without
fabricating percentages. A changed identity clears its old quota, advances the
profile revision and revokes old history grants; an unbound legacy report is
discarded on first authenticated binding. Unknown scope for the same identity
cannot clear confirmed exhaustion. Retry-After applies per profile across pools. Contradictory
positive/zero reports in one shared-group batch keep that group blocked. Only a
later fenced positive read recovers it. Errors and clock resets cannot clear
confirmed exhaustion. Balance uses the smallest remaining qualified window,
comparable epochs and a 30-second freshness limit; stale/unknown/API data supplies
no invented percentage.

Session v4 persists CLI Agent tabs/panes by run ID, retaining drafts/subscriptions
across remounts and domain moves. Legacy v1–v3 session bytes are backed up before
upgrading. Missing runs show a placeholder and never create a shell or dispatch.
Final-view close holds native and frontend send fences through Chat/Android
cleanup, releases on cancellation/failure and confirms process cleanup/save before
removal. Agent control cannot collectively move or close these protected runs.

Application close preparation freezes admission. Cancelling the guard leaves running work
alive. Confirmation and session save precede cancellation/drain; unresolved
cleanup or recovery writes keep Lomi open. Restart requires explicit recovery and
never restores profile binding or inference into an ordinary terminal.

## Plan status and outstanding work

| Stage          | Implemented                                                                                     | Remaining                                                                                              |
| -------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| P0             | Documentation/source review and explicit capability gates                                       | Real A/B login, refresh/logout, history/tool isolation and provider support qualification per adapter  |
| P1             | Shared store, profiles/pools, Settings, immutable credential journal                            | Complete provider auth/identity/logout adapters                                                        |
| P2             | Codex quota, principal groups, backoff, freshness and selection                                 | Model-specific quota scopes and other provider readers                                                 |
| P3             | Persisted text runs, per-attempt history, run tabs/session v4, Stop/drain and explicit recovery | Real native checkpoint/account and writable-effect qualification                                       |
| P4             | Codex empty-output text failover on typed exhaustion after drain; frozen history grants         | **Full coding-task failover, native cross-account continuity and other-provider exhaustion contracts** |
| P5             | Pure selection, new-terminal and managed-turn-boundary Codex balancing                          | Real-account acceptance and model-specific balance scope                                               |
| P6             | Regression fixtures, source review and local automated validation                               | Complete R01–R08/A01–A66, real accounts and platform/release qualification                             |
| Expanded scope | All 31 entries; twelve managed text adapters plus twenty macOS ARM64 Gateway families           | Qualify all implemented modes; remaining native auth/protocol/workflow gates                           |

Source implementation now includes mediated Codex tools and native gateway coding,
while real native thread/account continuity remains unqualified. Freebuff's current
headless interface and service permission, Trae release correlation, and SWE-agent
contained task deployment remain distinct gates. Junie documents EAP custom proxies
and LiteLLM URL/key flags; Droid documents BYOK base URLs. Released backend
correlation and startup/login/auxiliary routing admission remain unqualified.
These gaps and pending R01–R08/A01–A66 prevent declaring the expanded plan complete.

## Validation record

The current installer, Antigravity policy/usage and Pi transfer implementation
and source review finished before the test gate reopened. Local validation:

- `pnpm check` and `pnpm build`: passed.
- `pnpm test`: 250 host plus 35 AI-runtime cases passed.
- Mocked router/editor Playwright: 32 cases passed, including handoff consent,
  stale reviews, retained drafts and no task dispatch from transfer.
- Full locked Rust workspace: 1038 passed, 0 failed, 28 ignored.
- Installation receipt migration and idempotent reuse: passed. Isolated Kimi
  `2.1.1`, Kilo `7.8.3` and Pi `1.0.1` version probes passed with private homes,
  network/global writes denied and shared credential content inaccessible.

Strict production Clippy (`cargo clippy --manifest-path src-tauri/Cargo.toml
--locked -- -D warnings`) passed. The broader workspace variant also treats
vendored `snow` as a primary package and reports two existing `unused_mut`
warnings in its unchanged `resolvers/default.rs`; it did not pass `-D warnings`.
`pnpm tauri build --no-bundle` passed and produced the updated optimized macOS
ARM64 executable at `src-tauri/target/release/lomi`. No installer was published.
Targeted Prettier, Rust formatting and `git diff --check` passed. The packaged
Grok resource matches the compiled artifact manifest.
No configured profiles, real login or live paid qualification are available.

The following results are historical, from the previous supervised-native,
native-refresh and protocol-conversion phase. Its implementation and source review
finished before tests began, as instructed by the user. That local macOS ARM64
validation passed:

- `pnpm test`: 250 host tests and 35 AI-runtime tests; no failures.
- `pnpm check` and the production frontend build: passed.
- Mocked Playwright: 40 distinct cases passed (router, editor/coding effects,
  application-close and agent-terminal-profile). Native selection, explicit
  approval, preserved editor drafts, completion acknowledgement and the exact
  account/revision recovery bridge are covered; the rendered interaction was
  inspected.
- Full locked Rust workspace: 1022 passed, 0 failed, 28 ignored. Tests cover
  cross-protocol tools/SSE, account identity deduplication, private canonical
  checkpoint history, replaced workspaces and recovery authority retirement.
- Strict production Clippy (`-D warnings`), Rust formatting, targeted Prettier
  checks and `git diff --check`: passed without blanket lint suppression.
- `pnpm tauri build --no-bundle`: passed; produced that phase's optimized macOS
  ARM64 executable at `src-tauri/target/release/lomi`, with local router/runtime
  resources copied alongside it. No installer was published.
- Repository-wide Prettier reports three unchanged remote-runtime files:
  `packages/remote-terminal-runtime/src/index.mjs`, `src/model.mjs` and
  `tests/ipc.test.mjs`; router files pass their targeted check.

No live accounts are available to qualify API/subscription workflows for all
eight. Kimi/Kilo/Pi are now installed in the owned local tree and their isolated
version probes passed. Real-account,
installed-client, cancellation/approval/recovery and platform trials remain
pending; there is no complete or 100% all-provider automation claim. No login or
provider request was performed in the current phase.

The results below are **historical**, from the earlier same-family Gateway and
account-terminal phase. They do not validate the new source or fixtures.

Host: Darwin `27.0.0`, `arm64`; Lomi `0.5.6`. No real login, private credential read
or paid model invocation was performed by the implementation agents. Linux,
Windows and WSL are not qualified; private storage rejects unqualified non-Unix
platforms.

Per the user's instruction, implementation and source review finished before
final tests began. That historical validation pass completed:

- `pnpm test`: passed, including the AI runtime's 35 tests.
- `pnpm check` and `pnpm build`: passed, including SDK contract verification.
- Focused Explorer regression after the type-check fix: 7 tests passed,
  including retained standalone and split routed CLI views after rename/delete.
- Mocked browser coverage: all 33 distinct cases passed across the initial run
  and focused reruns (17 router, 7 editor/coding effects, 9 application close).
- `pnpm format:check` and `git diff --check`: passed.
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`: passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace --locked`:
  946 passed, 0 failed, 28 ignored. Ignored cases are not counted as passes.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --locked -- -D warnings`:
  passed without blanket lint suppression.
- `pnpm tauri build --no-bundle`: passed; produced the optimized macOS arm64
  executable at `src-tauri/target/release/lomi`. No installer was published.
- Focused native gateway PTY fixtures: passed after fixing the shutdown
  mutex/wait dependency and distinguishing Darwin's zombie-only groups from
  failed termination of live processes. These use local shell fixtures.

Provider-prefix mapping, immutable journal reopen and effective destination
deduplication regressions passed in the final Rust workspace run. Final source
review reported no remaining material findings. These checks do not qualify
installed provider clients or real cross-account continuity.
The [focused eight-CLI record](cli-router-focus-eight.md#validation) includes
native version/artifact probes and the saved-history/profile regression scope.

Primary references:

- [Codex environment configuration](https://learn.chatgpt.com/docs/config-file/environment-variables)
- [Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode)
- [Codex permissions](https://learn.chatgpt.com/docs/permissions)
- [Pinned Codex app-server source](https://github.com/openai/codex/tree/rust-v0.160.0/codex-rs/app-server)
- [Pinned Codex environment/tool policy](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/core/src/tools/spec_plan.rs)
- [Pinned OpenClaw execution](https://github.com/openclaw/openclaw/blob/v2026.9.8/src/commands/agent-exec.ts)
- [Pinned OpenCode startup and plugins](https://github.com/anomalyco/opencode/blob/aec0b9a6d8898f68f923aaf08b7306d931fd9d76/packages/opencode/src/plugin/index.ts)
- [Pinned Qwen Hosted Harness](https://github.com/QwenLM/qwen-code/blob/b12edec1401a28fc53cd9e714d5928b285071fc8/packages/cli/src/serve/hosted-harness-profile.ts)
- [Pinned Qwen managed store contract](https://github.com/QwenLM/qwen-code/blob/b12edec1401a28fc53cd9e714d5928b285071fc8/packages/core/src/managed-runtime/http-managed-session-store.ts)
- [Pinned native Kimi server protocol](https://github.com/MoonshotAI/kimi-code/blob/f67e6398fb3210ad8ace970e2dfd5bcc984ed61f/docs/en/reference/server-api.md)
- [Native Moonshot model and thinking contract](https://platform.kimi.ai/docs/guide/use-thinking-models)
- [Pinned Vibe public app-server](https://github.com/mistralai/mistral-vibe/tree/v2.25.8/vibe/app_server)
- [Claude Code headless usage](https://code.claude.com/docs/en/headless)
- [Claude Code authentication](https://code.claude.com/docs/en/authentication)
- [Claude Code legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)
- [Gemini CLI configuration](https://geminicli.com/docs/reference/configuration/)
- [Gemini CLI authentication](https://geminicli.com/docs/get-started/authentication/)
- [Gemini CLI headless mode](https://geminicli.com/docs/cli/headless/)
