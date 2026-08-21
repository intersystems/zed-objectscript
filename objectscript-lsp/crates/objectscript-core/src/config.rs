use serde::Deserialize;
use serde_json::{Map, Value};
/// Workspace/user configuration flags for the language server.
///
/// Deserialized from config input (with defaults via `#[serde(default)]`) and used to enable
/// or disable optional features such as snippets, formatting, linting, and strict mode.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// Enables completion items that include snippet text edits.
    #[serde(alias = "enable_snippets")]
    pub enable_snippets: bool,

    /// Enables document formatting support.
    #[serde(alias = "enable_formatting")]
    pub enable_formatting: bool,

    /// Enables lint/diagnostic checks.
    #[serde(alias = "enable_lint")]
    pub enable_lint: bool,

    /// Enables stricter parsing/diagnostic behavior when supported.
    #[serde(
        alias = "STRICT_MODE",
        alias = "strictMode",
        alias = "enable_strict_mode",
        alias = "strict_mode"
    )]
    pub enable_strict_mode: bool,
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

impl Config {
    /// Parse config from an LSP initializationOptions or didChangeConfiguration payload.
    ///
    /// The server accepts the direct ObjectScript config object, common client wrapper shapes,
    /// and flat VS Code-style keys:
    ///
    /// ```json
    /// { "enableStrictMode": true }
    /// { "objectscript": { "enableStrictMode": true } }
    /// { "objectscript.enableStrictMode": true }
    /// ```
    pub fn from_lsp_value(value: Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(normalize_lsp_config_value(value))
    }

    /// Parse config only when the LSP payload actually contains ObjectScript config keys.
    pub fn from_lsp_value_if_present(value: Value) -> Result<Option<Self>, serde_json::Error> {
        let normalized = normalize_lsp_config_value(value);
        if contains_config_key(&normalized) {
            serde_json::from_value(normalized).map(Some)
        } else {
            Ok(None)
        }
    }
}

fn normalize_lsp_config_value(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return Value::Object(Map::new());
    };

    for key in ["objectscript", "objectscriptLsp", "objectscript-lsp"] {
        if let Some(nested) = map.remove(key) {
            return nested;
        }
    }

    for key in ["initialization_options", "initializationOptions"] {
        if let Some(initialization_options) = map.remove(key) {
            return normalize_lsp_config_value(initialization_options);
        }
    }

    if let Some(settings) = map.remove("settings") {
        return normalize_lsp_config_value(settings);
    }

    if let Some(lsp) = map.remove("lsp").and_then(|value| match value {
        Value::Object(lsp) => Some(lsp),
        _ => None,
    }) {
        for key in ["objectscript-lsp", "objectscript_lsp", "objectscript"] {
            if let Some(server_config) = lsp.get(key) {
                if let Some(settings) = server_config.get("settings") {
                    return settings.clone();
                }
                if let Some(initialization_options) = server_config.get("initialization_options") {
                    return initialization_options.clone();
                }
                if let Some(initialization_options) = server_config.get("initializationOptions") {
                    return initialization_options.clone();
                }
                return server_config.clone();
            }
        }
    }

    let mut dotted = Map::new();
    for (key, value) in &map {
        if let Some(config_key) = key.strip_prefix("objectscript.") {
            dotted.insert(config_key.to_string(), value.clone());
        }
    }

    if dotted.is_empty() {
        Value::Object(map)
    } else {
        Value::Object(dotted)
    }
}

fn contains_config_key(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };

    const CONFIG_KEYS: [&str; 13] = [
        "enableSnippets",
        "enable_snippets",
        "enableFormatting",
        "enable_formatting",
        "enableLint",
        "enable_lint",
        "enableStrictMode",
        "enable_strict_mode",
        "strictMode",
        "strict_mode",
        "STRICT_MODE",
        "objectscript.enableStrictMode",
        "objectscript.enable_strict_mode",
    ];

    map.keys()
        .any(|key| CONFIG_KEYS.contains(&key.as_str()) || key.starts_with("objectscript."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_direct_camel_case_config() {
        let config = Config::from_lsp_value(json!({
            "enableStrictMode": false,
            "enableLint": false
        }))
        .expect("direct config should parse");

        assert!(!config.enable_strict_mode);
        assert!(!config.enable_lint);
        assert!(config.enable_formatting);
        assert!(config.enable_snippets);
    }

    #[test]
    fn parses_nested_objectscript_config() {
        let config = Config::from_lsp_value(json!({
            "objectscript": {
                "enableStrictMode": false
            }
        }))
        .expect("nested config should parse");

        assert!(!config.enable_strict_mode);
    }

    #[test]
    fn parses_flat_dotted_config() {
        let config = Config::from_lsp_value(json!({
            "objectscript.enableStrictMode": false
        }))
        .expect("flat dotted config should parse");

        assert!(!config.enable_strict_mode);
    }

    #[test]
    fn parses_legacy_strict_mode_aliases() {
        let upper = Config::from_lsp_value(json!({
            "STRICT_MODE": false
        }))
        .expect("upper-case alias should parse");
        let legacy = Config::from_lsp_value(json!({
            "strictMode": false
        }))
        .expect("legacy alias should parse");

        assert!(!upper.enable_strict_mode);
        assert!(!legacy.enable_strict_mode);
    }

    #[test]
    fn parses_zed_lsp_settings_shape() {
        let config = Config::from_lsp_value(json!({
            "lsp": {
                "objectscript-lsp": {
                    "initialization_options": {
                        "enableStrictMode": false
                    }
                }
            }
        }))
        .expect("zed lsp settings should parse");

        assert!(!config.enable_strict_mode);
    }

    #[test]
    fn parses_direct_initialization_options_wrapper() {
        let config = Config::from_lsp_value(json!({
            "initialization_options": {
                "enableStrictMode": false
            }
        }))
        .expect("direct initialization_options wrapper should parse");

        assert!(!config.enable_strict_mode);
    }

    #[test]
    fn parses_did_change_settings_wrapper() {
        let config = Config::from_lsp_value(json!({
            "settings": {
                "objectscript": {
                    "enableStrictMode": false
                }
            }
        }))
        .expect("settings wrapper should parse");

        assert!(!config.enable_strict_mode);
    }

    #[test]
    fn missing_config_keys_are_not_treated_as_config_updates() {
        let config = Config::from_lsp_value_if_present(json!({
            "binary": {
                "path": "/tmp/objectscript-lsp"
            }
        }))
        .expect("unknown lsp settings should be ignored");

        assert_eq!(config, None);
    }
}
