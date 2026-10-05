//! Configuration resolution.
//!
//! Precedence, highest first:
//!   1. CLI flags
//!   2. Environment variables
//!   3. `omniroute-zed.toml` in the current directory (optional)
//!   4. Built-in defaults
//!
//! The API key is *not* resolved here beyond presence-checking: Zed itself
//! reads `OMNIROUTE_API_KEY` for the `omniroute` OpenAI-compatible provider
//! (it derives the variable name from the provider id). We only mirror that
//! key into Zed's settings when the user explicitly asks us to, because Zed
//! prefers the keychain and we never want to write a secret into a plaintext
//! settings file by surprise.

use std::path::PathBuf;
use std::time::Duration;

/// Built-in default base URL of the OmniRoute gateway.
pub const DEFAULT_BASE_URL: &str = "http://localhost:20128";
/// Default seconds between syncs (5 minutes).
pub const DEFAULT_SYNC_INTERVAL_SECS: u64 = 300;
/// Default Zed provider id. Zed maps this to `OMNIROUTE_API_KEY`.
pub const DEFAULT_PROVIDER_ID: &str = "omniroute";

/// Fully-resolved configuration for one run.
#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the OmniRoute gateway, without a trailing slash.
    pub base_url: String,
    /// Seconds between syncs in watch mode.
    pub sync_interval: Duration,
    /// Zed provider id used as the `language_models.openai_compatible` key.
    pub provider_id: String,
    /// OmniRoute API key, when one is available from the environment.
    pub api_key: Option<String>,
    /// Path to Zed's `settings.json`. `None` means "discover it".
    pub settings_path: Option<PathBuf>,
    /// Whether to write the API key into Zed settings (off by default).
    pub write_api_key: bool,
}

impl Config {
    /// URL of the models endpoint, e.g. `http://localhost:20128/v1/models`.
    pub fn models_url(&self) -> String {
        format!("{}/v1/models", self.base_url)
    }

    /// URL that Zed's OpenAI-compatible provider needs as `api_url`.
    ///
    /// Zed appends `/chat/completions` to whatever we put here (see
    /// `open_ai.rs`: `.uri(format!("{api_url}/chat/completions"))`), so this
    /// must be the *root* of the API, i.e. `http://localhost:20128/v1`.
    pub fn zed_api_url(&self) -> String {
        format!("{}/v1", self.base_url)
    }

    /// `Authorization` header value when a key is available.
    pub fn auth_header(&self) -> Option<String> {
        self.api_key.as_ref().map(|key| format!("Bearer {key}"))
    }
}

/// Parses a positive duration from seconds, rejecting zero.
pub fn parse_interval_secs(raw: &str) -> Result<u64, String> {
    let secs: u64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("invalid interval `{raw}`: expected whole seconds"))?;
    if secs == 0 {
        return Err("invalid interval `0`: must be at least 1 second".to_string());
    }
    Ok(secs)
}

/// Trims trailing slashes so URL joining never doubles up.
pub fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("base URL is empty".to_string());
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(format!(
            "base URL `{trimmed}` must start with http:// or https://"
        ));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_trailing_slashes() {
        assert_eq!(
            normalize_base_url("http://localhost:20128///").unwrap(),
            "http://localhost:20128"
        );
        assert_eq!(
            normalize_base_url("  http://localhost:20128/  ").unwrap(),
            "http://localhost:20128"
        );
    }

    #[test]
    fn rejects_urls_without_scheme() {
        assert!(normalize_base_url("localhost:20128").is_err());
        assert!(normalize_base_url("").is_err());
        assert!(normalize_base_url("   ").is_err());
    }

    #[test]
    fn builds_endpoints_from_base_url() {
        let config = Config {
            base_url: "http://localhost:20128".to_string(),
            sync_interval: Duration::from_secs(300),
            provider_id: DEFAULT_PROVIDER_ID.to_string(),
            api_key: None,
            settings_path: None,
            write_api_key: false,
        };
        // Zed appends /chat/completions, so api_url must stop at /v1.
        assert_eq!(config.zed_api_url(), "http://localhost:20128/v1");
        assert_eq!(config.models_url(), "http://localhost:20128/v1/models");
    }

    #[test]
    fn rejects_non_positive_intervals() {
        assert!(parse_interval_secs("0").is_err());
        assert!(parse_interval_secs("-5").is_err());
        assert!(parse_interval_secs("abc").is_err());
        assert!(parse_interval_secs("").is_err());
    }

    #[test]
    fn accepts_positive_intervals() {
        assert_eq!(parse_interval_secs("300").unwrap(), 300);
        assert_eq!(parse_interval_secs("  60  ").unwrap(), 60);
    }

    #[test]
    fn auth_header_absent_without_key() {
        let config = Config {
            base_url: "http://localhost:20128".to_string(),
            sync_interval: Duration::from_secs(300),
            provider_id: DEFAULT_PROVIDER_ID.to_string(),
            api_key: None,
            settings_path: None,
            write_api_key: false,
        };
        assert!(config.auth_header().is_none());
    }
}
