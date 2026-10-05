# CLI agent integrations

Run installed agents in ordinary Lomi terminal panels. Lomi recognizes the 16
agents below on macOS and Linux. It recognizes
native executables, documented Node/Bun launchers and Python console scripts;
inline code, prompt arguments and remote SSH commands are not agent identities.
This does not install the agents or restore their conversations after restart.

The titlebar **+ → Agents** dialog launches 15 clients in this catalog. Pi can
be started with a terminal command or through Router.
It shows only executables found in the selected terminal environment and uses
bundled brand icons from Iconify. Choose a CLI and the number of terminal panels
(four by default). Lomi opens one new tab with a grid of panels and starts an
independent CLI process in each panel, in the current project folder. Four panels
use a two-by-two grid. Refresh the list after installing a CLI. Launching does not
change client configuration. Reopening a saved session preserves the panel layout
and starts ordinary shells without relaunching the agents.
The launcher supports local Bash, Zsh, Fish and POSIX sh on macOS and Linux;
Windows, PowerShell and WSL launch environments are not supported yet.

Settings → Agent control → MCP clients lists Claude, Codex, Gemini, Copilot,
Cursor, OpenCode, OpenClaw, Hermes, Kilo, Qwen, Kiro, Vibe, Kimi, Grok and
Antigravity only when their executables are found in the default supported local
shell environment. Existing configuration files alone do not count as an
installation. Refresh the list after installing or removing a CLI.
**Install** registers Lomi in the selected user configuration, and
**Install for all supported clients** applies only to installed clients. Local running agents also offer
applicable integrations in the status bar. Inspection never writes configuration;
installation requires a click. Client approval and Lomi pairing still apply.

## Router setup

Open **Settings → Agent control → Router**, select **New router**, choose an
agent, then add or include its saved accounts in priority order. Accounts are
saved separately and can be reused. New routers start disabled; enable one only
after configuring compatible accounts. Existing routers open directly for
editing. API endpoint and key changes require explicit saving; subscription
logins stay in the native CLI. Quota balancing uses comparable fresh reports and
falls back to account order when they are unavailable.

On macOS ARM64 with Node 22.19 or newer, `node scripts/install-router-clis.mjs`
prepares the exact Kimi, Kilo and Pi packages pinned in
`scripts/router-cli-installations.json`, without running lifecycle scripts or
changing shell startup files. It requires an existing trusted `~/.local/bin/npm`
file or symlink and checks ownership and permissions for Node, npm and their
parent directories. Add `~/.local/bin` to the launch environment's PATH
if needed. Native adapters and `cli_router/gateway_profiles.rs` enforce supported
versions and capabilities. Source support does not establish real-account
qualification, automatic subscription failover or cross-account native history
continuity. Saved runs never start inference during restoration.

## MCP adapters

Paths below are defaults. Documented environment overrides are resolved from the
running process for status-bar setup and from Lomi's environment for Settings.
Project settings may override user settings. For newly supported agents started
with an explicit configuration/profile flag, use that agent's own setup or
Settings for its default user configuration; Lomi does not guess the active file.

| Agent                                                                                                          | Command            | Automatic user MCP configuration                                                            |
| -------------------------------------------------------------------------------------------------------------- | ------------------ | ------------------------------------------------------------------------------------------- |
| Claude Code                                                                                                    | `claude`           | `~/.claude.json`                                                                            |
| OpenAI Codex CLI                                                                                               | `codex`            | `~/.codex/config.toml`                                                                      |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md)                   | `gemini`           | `~/.gemini/settings.json`                                                                   |
| [GitHub Copilot CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers) | `copilot`          | `~/.copilot/mcp-config.json`; local server type and tools list                              |
| Cursor CLI                                                                                                     | `cursor-agent`     | `~/.cursor/mcp.json`                                                                        |
| [OpenCode](https://opencode.ai/v2/docs/mcp-servers)                                                            | `opencode`         | `~/.config/opencode/opencode.json` or `.jsonc`; current v2 `mcp.servers` with command array |
| [OpenClaw](https://docs.openclaw.ai/gateway/config-extensions)                                                 | `openclaw`         | `~/.openclaw/openclaw.json`; JSON5 `mcp.servers`                                            |
| [Hermes Agent](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/mcp.md) | `hermes`           | `~/.hermes/config.yaml`; `mcp_servers` map                                                  |
| [Pi Coding Agent](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md)              | `pi`               | Requires an MCP extension; no built-in MCP registry                                         |
| [Kilo Code CLI](https://kilo.ai/docs/automate/mcp/using-in-kilo-code)                                          | `kilo`, `kilocode` | `~/.config/kilo/kilo.jsonc`; `mcp` with command array                                       |
| [Qwen Code](https://qwenlm.github.io/qwen-code-docs/en/users/features/mcp/)                                    | `qwen`             | `~/.qwen/settings.json`                                                                     |
| [Kiro CLI](https://kiro.dev/docs/mcp/configuration/)                                                           | `kiro-cli`         | `~/.kiro/settings/mcp.json`                                                                 |
| [Mistral Vibe](https://docs.mistral.ai/vibe/code/cli/mcp-servers)                                              | `vibe`             | `~/.vibe/config.toml`; `[[mcp_servers]]` entries                                            |
| [Kimi Code CLI](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/mcp.md)                | `kimi`             | `~/.kimi-code/mcp.json`; targets the successor, not legacy Kimi CLI                         |
| [Grok Build](https://docs.x.ai/build/features/mcp-servers)                                                     | `grok`             | `~/.grok/config.toml`; `mcp_servers` table                                                  |
| [Antigravity CLI](https://antigravity.google/docs/mcp/)                                                        | `agy`              | `~/.gemini/config/mcp_config.json`                                                          |

Supported environment overrides include `CODEX_HOME`, `GEMINI_CLI_HOME`,
`COPILOT_HOME`, `XDG_CONFIG_HOME` (OpenCode and Kilo), `OPENCODE_CONFIG`,
`OPENCLAW_CONFIG_PATH`, `OPENCLAW_STATE_DIR`, `HERMES_HOME`, `KILO_CONFIG`,
`QWEN_HOME`, `KIRO_HOME`, `VIBE_HOME`, `KIMI_CODE_HOME` and `GROK_HOME`.
`GEMINI_CLI_HOME` is the parent of `.gemini`. Relative overrides are rejected.

Antigravity CLI's current MCP documentation retains the global
`~/.gemini/config/mcp_config.json` path and also supports workspace-local
`.agents/mcp_config.json`. Lomi's automatic setup registers the global server.
A blank Antigravity MCP file is treated as uninitialized configuration:
inspection leaves it untouched, and explicit installation initializes the
`mcpServers` object while backing up the exact original file. Nonblank invalid
JSON still blocks automatic setup. Remote Antigravity servers use `serverUrl`;
Lomi registers a local `command` and `args` entry.

OpenCode configurations with both JSON and JSONC files require manual setup.
Legacy OpenCode MCP maps require migration to v2 or manual setup.
Inline OpenCode/Kilo configuration, OpenCode/Kilo's additional config directory,
custom OpenClaw profiles/homes and custom Claude MCP homes are not guessed.
Vibe with `VIBE_CLI=rust` requires manual qualification; the upstream Rust MCP
manager documents OAuth-only additions.

Configuration updates preserve unrelated values, reject a foreign server named
`lomi`, check file revisions, save exact original backups and replace files
atomically. JSON/JSONC/JSON5 edits preserve unrelated comments and text. YAML
updates reformat the file and remove comments; the original remains in the backup.
YAML aliases, merge keys, unsupported tags and duplicate keys are rejected rather
than silently rewritten. Parsing is bounded, with no include-file loading or
property expansion by Lomi.

## Titles and notifications

All terminal panels display titles and supported terminal notification sequences
emitted by their programs. Automatic title setup is available for Codex, Claude,
Cursor, Antigravity, Gemini and Qwen. Gemini/Qwen retain their enabled defaults;
Lomi offers setup only when title settings are disabled. Gemini uses
`ui.hideWindowTitle` / `ui.dynamicWindowTitle`, and Qwen uses
`ui.hideWindowTitle` / `ui.showStatusInTitle`. Their
[Gemini schema](https://github.com/google-gemini/gemini-cli/blob/main/schemas/settings.schema.json)
and [Qwen settings](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/)
define these fields.

Automatic notification setup remains available for Codex and Claude. Other
agents may already send terminal bells/titles or provide their own desktop
notifications. Their notification hooks are not installed by Lomi. No agent is
restarted automatically after configuration changes.

Lomi stores supported terminal notifications in a local inbox, available from
the titlebar account menu under **Notifications**, including while signed out.
New entries do not open a panel or show an in-app alert. The signed-in avatar
shows the unread count, capped at `9+`; while signed out, the count appears only
in the menu. Open the inbox to read or dismiss individual entries, or use
**Clear all notifications** to remove every entry, including unread ones.
Opening the inbox alone leaves unread state unchanged. The latest 200
entries and their read state survive application restarts.

Inbox rows show the agent icon, notification title, source, context and relative time.
Clicking a row marks it read; the dismiss control appears on hover or keyboard
focus, and remains visible on touch screens. Project/workspace/tab context wraps
beside the source and is also available in the row tooltip.
Explicit Claude hooks identify Claude Code. Other terminal notifications use
the natively recognized foreground CLI when available, otherwise a generic
terminal icon. Agent identity is saved with the entry, so later process changes
do not change old notifications. Existing history without source metadata keeps
its generic icon, except for the two known historical Claude Code titles.

Focused-window events still enter the inbox. Lomi requests system alerts while
the main window is in the background; OS notification settings determine actual
delivery. Denied system permission does not discard the inbox entry. The
**Agent notifications** switch in **Settings → Terminal** disables both new
inbox entries and system alerts without deleting existing history. OSC 9
messages use generic agent wording and never expose their raw terminal text.

## Manual MCP clients

Start Lomi's MCP server in Settings and use **MCP JSON configuration for this
running instance** as the source of the executable and argument values. That
manual connection uses the separate `lomi-mcp` helper and must be copied again
after a server restart, as described in [MCP usage](mcp/USAGE.md).

Pi needs a separately installed MCP extension. The other 15 catalog clients
support automatic MCP configuration.

## Account usage in the titlebar

When a recognized agent runs in a local Lomi terminal, a compact usage control
appears beside the account/settings control. It includes agents in background
tabs and workspaces. The percentage means **remaining account quota**, using the
most restrictive reported window. Click or right-click it to see each CLI account's
windows, reset times, last successful update and availability. Keyboard users can
open it with Enter or Arrow Down and dismiss it with Escape.

The details merge running sessions with the same verified account key into one
entry per CLI. Different accounts remain separate even when their quota values
match. Codex uses the selected ChatGPT workspace and user identity from its
[native token claims](https://github.com/openai/codex/blob/ca466061d64f0b44f416135c7fd06aa7af850bbc/codex-rs/login/src/token_data.rs),
so different credential stores and token revisions can share one entry while
different workspace members remain separate. Missing or mismatched identity
claims fall back to the credential fingerprint. Other HTTP readers use the saved
credential fingerprint, including the selected Cursor team or Kimi region; separate logins
with different tokens remain separate when no verified account ID is available.
Antigravity merges only identical native report contexts because it manages its
own authentication. Unknown account identities remain separate. Only opaque,
process-local keys reach the interface; credentials and account IDs stay native.

Lomi checks usage every 15 seconds while agents are running, and when the window
regains focus or the details menu opens. Refresh requests share an in-flight
read and native cache for the same account or process context. Provider throttling
delays further requests, including manual refreshes. A transient HTTP service
error retains the last successful values with an explicit stale indication;
signing out or changing credentials does not carry another account's values
forward.

Account quota and a conversation's context window are different. Lomi does not
estimate subscription quota from tokens, prompts, transcripts or terminal output.
All agents in the catalog can appear in this control. Numeric quota requires a
provider-specific reader and a compatible existing login. Provider-dependent
clients such as OpenCode do not have one universal account limit;
the details show an unavailable status when Lomi has no verified account reader.
This describes Lomi's current coverage, rather than assuming the provider has no
usage API.

| Reader          | Existing login                                  | Reported quota                                                               |
| --------------- | ----------------------------------------------- | ---------------------------------------------------------------------------- |
| Codex           | ChatGPT account in the running CLI's Codex home | Primary and secondary account windows, including additional reported buckets |
| Claude Code     | Claude account OAuth with profile access        | Five-hour and supported weekly subscription/model windows                    |
| Cursor CLI      | The CLI's existing OAuth credential store       | Included plan usage for the current billing period                           |
| Kimi Code       | Existing Kimi Code OAuth credentials            | Five-hour, weekly and monthly windows reported by the account                |
| Antigravity CLI | The running CLI's existing account login        | Shared model-group quota buckets and their reported reset times              |

These readers follow [Codex's backend client](https://github.com/openai/codex/blob/main/codex-rs/backend-client/src/client.rs),
Claude's account usage response, the installed Cursor CLI's DashboardService
`GetCurrentPeriodUsage` contract and [Kimi Code's managed usage reader](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/src/managed-usage.ts).
API-key and custom-provider accounts are not assumed to share a saved OAuth
subscription. The HTTP readers report login expiry without modifying the CLI's
authentication.

Antigravity uses the installed CLI's supported `-p /usage --output-format json`
report. Its [CLI changelog](https://antigravity.google/docs/changelog/)
documents this read-only command from version 1.1.11: it does not start an agent
turn, spend model quota or create a conversation. Lomi checks the executable's
version before invoking it and accepts compatible stable 1.x versions. The query
uses the running process's account context, reads shared group/bucket limits and
refreshes on the same titlebar schedule. Its process-context cache lasts at most
five seconds after a successful read. Command failures clear numeric values;
Lomi cannot inspect the CLI-managed authentication to qualify an old account's
saved quota. API-key mode remains unavailable. See the
[headless output contract](https://antigravity.google/docs/cli/headless/) and
[model quota panel](https://antigravity.google/docs/cli/commands/usage/).

The Kimi reader uses the current native Kimi Code client's selected managed
provider, regional endpoint and file credentials in `KIMI_CODE_HOME` (by default
`~/.kimi-code`). The legacy Python client uses a separate store and reports
unavailable; Lomi does not substitute a different client's saved account.

Gemini CLI remains recognized, with usage available in the CLI itself. Lomi does
not reuse its OAuth credentials to call Code Assist: Google's [Gemini CLI terms
notice](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/tos-privacy.md)
prohibits that direct access from third-party software.

Usage requests and credential reads run natively. Credentials never enter the
webview, and Lomi does not modify CLI configuration, initiate login, restart
terminals or send model requests. The Antigravity command handles its own normal
credential and quota-cache maintenance. Only the trusted main app can inspect
usage for a verified CLI process in one of its own terminal sessions. Custom
providers, API-key authentication and expired logins may not expose plan quota.

Some providers expose usage through private client endpoints rather than stable
public APIs. Their response formats and availability can change independently of
Lomi; errors appear in the details without inventing a percentage. Native tests
use isolated processes and credentials; UI tests use provider response fixtures.

On macOS, `node --experimental-strip-types tests/native/run-agent-usage-smoke.mjs`
exercises the native command, process ownership, titlebar menu, terminal continuity and settings-window
denial in an isolated app profile. Its optional `--live-all` probe reads the
existing Codex, Claude, Cursor and current Kimi Code accounts using inert process
fixtures. It reports availability without recording credentials, launching agent
sessions or sending model requests.
The separate `--live-agy` mode verifies the Antigravity read-only report through
an inert native fixture and the installed CLI, without recording quota values.

## Qualification

The title, notification and MCP adapters follow official documentation/source
reviewed on 2026-09-24. The account usage readers were reviewed on 2026-09-29.
Automated tests cover registration formats, preservation/conflicts, process
identification, environment paths and the opt-in Settings/status-bar workflow.
This is not an end-to-end qualification of installed/authenticated versions of
all agents. Process inspection is macOS/Linux only; WSL and Windows agent
inspection are not added by this change. Existing generic terminal behavior is
unchanged on those platforms.

Antigravity usage was qualified on macOS ARM64 with agy 1.2.13. The native
fixture verified automatic percentage changes in the titlebar and details menu,
reset times and stopped-process rejection. The live read returned four numeric
quota windows and passed the zero-agent-turn response guard. Other platforms
were not independently qualified for this reader.
