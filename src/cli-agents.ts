export const cliNames = {
  codex: "Codex",
  agy: "Antigravity CLI",
  cursor: "Cursor CLI",
  claude: "Claude Code",
  gemini: "Gemini CLI",
  copilot: "GitHub Copilot CLI",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
  hermes: "Hermes Agent",
  pi: "Pi Coding Agent",
  kilo: "Kilo Code CLI",
  qwen: "Qwen Code",
  kiro: "Kiro CLI",
  vibe: "Mistral Vibe",
  kimi: "Kimi Code CLI",
  grok: "Grok Build",
} as const;

export type CliAgent = keyof typeof cliNames;
