//! HTTP client for the OmniRoute gateway.
//!
//! Deliberately blocking: the syncer is a short-lived host process with a
//! sleep loop, so pulling in an async runtime would add weight without
//! buying anything.

use std::time::Duration;

use crate::config::Config;
use crate::models::{ModelsResponse, to_zed_model};

/// How long to wait on the models endpoint end-to-end. OmniRoute is normally
/// localhost, but the base URL is configurable, so a slow remote must not hang.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Cap on name resolution. Generous, because a remote base URL may sit behind
/// a slow DNS, but bounded so a dead resolver surfaces as an error.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
/// Cap on establishing the socket.
///
/// This is the one that matters for `http://localhost:20128`: on Windows,
/// `localhost` resolves to `::1` first and the gateway listens on IPv4 only.
/// The `::1` connect stalls for ~2s before refusing, so without a connect
/// timeout that dead address family can consume the entire request budget
/// before the IPv4 fallback is ever tried.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Builds the HTTP agent with per-phase timeouts.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .timeout_resolve(Some(RESOLVE_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .build()
        .into()
}

/// Error from talking to OmniRoute.
#[derive(Debug)]
pub enum ClientError {
    /// Transport-level failure (connection refused, DNS, timeout).
    Transport(String),
    /// Non-2xx response.
    Status { status: u16, body: String },
    /// Response body was not the JSON we expected.
    Decode(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(message) => write!(formatter, "request failed: {message}"),
            Self::Status { status, body } => {
                let trimmed: String = body.chars().take(200).collect();
                write!(formatter, "HTTP {status}: {trimmed}")
            }
            Self::Decode(message) => write!(formatter, "could not decode response: {message}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Ordering for the emitted model list.
///
/// OmniRoute does not guarantee a stable order between `/v1/models` calls.
/// Writing that order straight through would rewrite Zed's settings file on
/// every sync, and Zed watches that file — so a 5-minute watch would trigger
/// pointless reloads. Sorting by id makes the output deterministic: Zed only
/// reloads when the model *set* actually changes.
///
/// Within a provider family (e.g. `auto/…`, `claude/…`) ids sort descending,
/// so `auto/reasoning` comes before `auto/chat`.
fn sort_key(id: &str) -> (String, std::cmp::Reverse<String>) {
    match id.split_once('/') {
        Some((prefix, rest)) => (prefix.to_string(), std::cmp::Reverse(rest.to_string())),
        None => (String::new(), std::cmp::Reverse(id.to_string())),
    }
}

/// Fetches `/v1/models` and maps it into Zed's model schema.
///
/// The result is sorted deterministically; see [`sort_key`].
pub fn fetch_models(config: &Config) -> Result<Vec<crate::models::ZedModel>, ClientError> {
    let url = config.models_url();
    let mut request = agent().get(&url);

    if let Some(header) = config.auth_header() {
        request = request.header("Authorization", &header);
    }
    if let Some(key) = config.api_key.as_deref() {
        // Some OmniRoute deployments accept the Anthropic-style header too.
        request = request.header("x-api-key", key);
    }

    let mut response = request
        .call()
        .map_err(|err| ClientError::Transport(err.to_string()))?;

    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| ClientError::Transport(err.to_string()))?;

    if !(200..300).contains(&status) {
        return Err(ClientError::Status { status, body });
    }

    let parsed: ModelsResponse =
        serde_json::from_str(&body).map_err(|err| ClientError::Decode(err.to_string()))?;

    let mut models: Vec<crate::models::ZedModel> = parsed.data.iter().map(to_zed_model).collect();
    models.sort_by_key(|model| sort_key(&model.name));
    Ok(models)
}

/// A short-lived health probe used by the `status` subcommand.
pub fn probe(config: &Config) -> Result<usize, ClientError> {
    Ok(fetch_models(config)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::time::Duration;

    fn test_config(base_url: &str) -> Config {
        Config {
            base_url: base_url.to_string(),
            sync_interval: Duration::from_secs(300),
            provider_id: "omniroute".to_string(),
            api_key: None,
            settings_path: None,
            write_api_key: false,
        }
    }

    #[test]
    fn reports_connection_refused_as_transport_error() {
        // Port 1 is reserved and nothing listens there.
        let config = test_config("http://127.0.0.1:1");
        match fetch_models(&config) {
            Err(ClientError::Transport(_)) => {}
            other => panic!("expected a transport error, got {other:?}"),
        }
    }

    #[test]
    fn status_error_is_rendered_with_truncated_body() {
        let error = ClientError::Status {
            status: 401,
            body: "x".repeat(500),
        };
        let rendered = error.to_string();
        assert!(rendered.starts_with("HTTP 401: "));
        // Body must be truncated so a huge HTML error page cannot flood logs.
        assert!(rendered.len() < 260);
    }

    #[test]
    fn sort_is_deterministic_and_groups_by_prefix() {
        // Flat ids carry an empty prefix, so they sort first; prefixed
        // families then group together alphabetically.
        let ids = ["claude/x", "auto/z", "auto/a", "gpt-5", "auto/m"];
        let mut sorted = ids.to_vec();
        sorted.sort_by_key(|id| sort_key(id));
        assert_eq!(sorted, ["gpt-5", "auto/z", "auto/m", "auto/a", "claude/x"]);
    }

    #[test]
    fn equal_prefixes_sort_descending() {
        let ids = ["auto/coding:pro", "auto/chat", "auto/reasoning"];
        let mut sorted = ids.to_vec();
        sorted.sort_by_key(|id| sort_key(id));
        assert_eq!(sorted, ["auto/reasoning", "auto/coding:pro", "auto/chat"]);
    }

    #[test]
    fn sort_key_distinguishes_ids_with_a_shared_prefix() {
        // Guards the ordering rule itself: if the sort collapsed `auto/chat`
        // and `auto/reasoning` into one key, the output would be unstable.
        assert_ne!(sort_key("auto/chat"), sort_key("auto/reasoning"));
        assert_eq!(sort_key("auto/chat"), sort_key("auto/chat"));
    }
}
