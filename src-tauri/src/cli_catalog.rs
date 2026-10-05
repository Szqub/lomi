use serde::{Deserialize, Serialize};

// These identifiers are recognized only while migrating persisted records.
// They are deliberately absent from TitleCli and every operational catalog.
pub(crate) fn is_retired_id(id: &str) -> bool {
    matches!(
        id,
        "aider"
            | "cline"
            | "goose"
            | "droid"
            | "openhands"
            | "continue"
            | "amp"
            | "auggie"
            | "crush"
            | "interpreter"
            | "junie"
            | "freebuff"
            | "sweagent"
            | "deepagents"
            | "trae"
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum TitleCli {
    Codex,
    Agy,
    Cursor,
    Claude,
    Gemini,
    Copilot,
    Opencode,
    Openclaw,
    Hermes,
    Pi,
    Kilo,
    Qwen,
    Kiro,
    Vibe,
    Kimi,
    Grok,
}

impl TitleCli {
    pub(crate) const MCP_CLIENTS: [Self; 15] = [
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::Copilot,
        Self::Cursor,
        Self::Opencode,
        Self::Openclaw,
        Self::Hermes,
        Self::Kilo,
        Self::Qwen,
        Self::Kiro,
        Self::Vibe,
        Self::Kimi,
        Self::Grok,
        Self::Agy,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Agy => "Antigravity CLI",
            Self::Cursor => "Cursor CLI",
            Self::Claude => "Claude Code",
            Self::Gemini => "Gemini CLI",
            Self::Copilot => "GitHub Copilot CLI",
            Self::Opencode => "OpenCode",
            Self::Openclaw => "OpenClaw",
            Self::Hermes => "Hermes Agent",
            Self::Pi => "Pi Coding Agent",
            Self::Kilo => "Kilo Code CLI",
            Self::Qwen => "Qwen Code",
            Self::Kiro => "Kiro CLI",
            Self::Vibe => "Mistral Vibe",
            Self::Kimi => "Kimi Code CLI",
            Self::Grok => "Grok Build",
        }
    }

    pub(crate) fn supports_titles(self) -> bool {
        matches!(
            self,
            Self::Codex | Self::Agy | Self::Cursor | Self::Claude | Self::Gemini | Self::Qwen
        )
    }
}
