use serde::Deserialize;
/// Workspace/user configuration flags for the language server.
///
/// Deserialized from config input (with defaults via `#[serde(default)]`) and used to enable
/// or disable optional features such as snippets, formatting, linting, and strict mode.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Enables completion items that include snippet text edits.
    pub enable_snippets: bool,

    /// Enables document formatting support.
    pub enable_formatting: bool,

    /// Enables lint/diagnostic checks.
    enable_lint: bool,

    /// Enables stricter parsing/diagnostic behavior when supported.
    enable_strict_mode: bool,
}

impl Default for Config {
    /// Returns the default configuration (all features enabled by default).
    fn default() -> Self {
        Self {
            enable_strict_mode: true,
            enable_formatting: true,
            enable_lint: true,
            enable_snippets: true,
        }
    }
}
