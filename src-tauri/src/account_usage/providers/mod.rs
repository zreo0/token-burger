pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod github_copilot;

use crate::account_usage::AccountUsageProvider;

pub fn all_providers() -> Vec<Box<dyn AccountUsageProvider>> {
    vec![
        Box::new(codex::CodexUsageProvider),
        Box::new(claude_code::ClaudeCodeUsageProvider),
        Box::new(cursor::CursorUsageProvider),
        Box::new(github_copilot::GithubCopilotUsageProvider),
    ]
}
