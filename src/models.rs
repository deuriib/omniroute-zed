//! Mapping OmniRoute's `/v1/models` payload onto Zed's
//! `language_models.openai_compatible.*.available_models` schema.
//!
//! Both shapes were read from Zed's source rather than guessed:
//!
//! ```text
//! OpenAiCompatibleSettingsContent { api_url, available_models, custom_headers }
//! OpenAiCompatibleAvailableModel  { name, display_name, max_tokens,
//!                                  max_output_tokens, max_completion_tokens,
//!                                  reasoning_effort, capabilities }
//! OpenAiCompatibleModelCapabilities { tools, images, parallel_tool_calls,
//!                                     prompt_cache_key, chat_completions,
//!                                     interleaved_reasoning, max_tokens_parameter }
//! ```
//!
//! `max_tokens` is required by Zed, but 340 of the 1192 models OmniRoute
//! serves report no context window. We substitute a conservative default
//! rather than dropping those models from Zed's picker.

use serde::{Deserialize, Serialize};

/// Conservative fallback context window for models that report none.
pub const DEFAULT_MAX_TOKENS: u64 = 128_000;

/// One entry of OmniRoute's `data` array.
#[derive(Debug, Clone, Deserialize)]
pub struct OmniRouteModel {
    /// Model identifier, e.g. `auto/cheap` or `claude/...`.
    pub id: String,
    /// Optional human-friendly name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Total context window. Absent on many OmniRoute entries.
    #[serde(default)]
    pub context_length: Option<u64>,
    /// Max output tokens, when advertised.
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    /// Modality hints, e.g. `["text", "image"]`.
    #[serde(default)]
    pub input_modalities: Vec<String>,
    /// Optional display hints, present on some entries.
    #[serde(default)]
    pub display_hint: Option<DisplayHint>,
    /// Capability flags.
    #[serde(default)]
    pub capabilities: Option<OmniRouteCapabilities>,
}

/// Non-essential presentation hints OmniRoute may attach to a model.
#[derive(Debug, Clone, Deserialize)]
pub struct DisplayHint {
    #[serde(default)]
    pub title: Option<String>,
}

/// Capability flags OmniRoute reports.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OmniRouteCapabilities {
    #[serde(default)]
    pub tool_calling: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub thinking: bool,
    /// Reasoning-effort tiers, e.g. `["low","medium","high","xhigh","max"]`.
    #[serde(default)]
    pub effort_tiers: Vec<String>,
}

/// The `/v1/models` response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelsResponse {
    #[serde(default)]
    pub data: Vec<OmniRouteModel>,
}

/// Zed's `reasoning_effort` enum, lowercase-serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ReasoningEffort {
    /// Parses an OmniRoute effort tier, dropping unknown tiers.
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" | "extra-high" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// Sort rank so a deterministic default can be chosen from a tier list.
    fn rank(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Minimal => 1,
            Self::Low => 2,
            Self::Medium => 3,
            Self::High => 4,
            Self::XHigh => 5,
            Self::Max => 6,
        }
    }
}

/// Zed's `capabilities` object for a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedCapabilities {
    pub tools: bool,
    pub images: bool,
    pub parallel_tool_calls: bool,
    pub prompt_cache_key: bool,
    pub chat_completions: bool,
    pub interleaved_reasoning: bool,
    pub max_tokens_parameter: bool,
}

/// One entry in Zed's `available_models` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedModel {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    pub capabilities: ZedCapabilities,
}

/// Whether a model accepts image input.
fn accepts_images(model: &OmniRouteModel) -> bool {
    model
        .input_modalities
        .iter()
        .any(|m| m.eq_ignore_ascii_case("image"))
}

/// Picks Zed's default reasoning effort from OmniRoute's effort tiers.
///
/// Zed reads `reasoning_effort` as "this model thinks by default at this level",
/// so we choose the *highest* tier the model advertises. Returning `None`
/// means "no thinking", which Zed treats as a plain non-reasoning model.
fn default_reasoning_effort(model: &OmniRouteModel) -> Option<ReasoningEffort> {
    let caps = model.capabilities.as_ref()?;
    if !caps.reasoning && !caps.thinking {
        return None;
    }
    caps.effort_tiers
        .iter()
        .filter_map(|tier| ReasoningEffort::parse(tier))
        .max_by_key(|effort| effort.rank())
}

/// Best-effort display name.
///
/// Preference order: explicit `display_name`, then a humanized id, then the
/// subtitle hint if present.
fn display_name_for(model: &OmniRouteModel) -> Option<String> {
    if let Some(name) = model
        .display_name
        .as_ref()
        .filter(|name| !name.trim().is_empty())
    {
        return Some(name.clone());
    }

    if let Some(hint) = model.display_hint.as_ref()
        && let Some(title) = hint.title.as_ref().filter(|t| !t.trim().is_empty())
    {
        return Some(title.clone());
    }

    let leaf = model.id.rsplit('/').next().unwrap_or(&model.id);
    if leaf.is_empty() || leaf == model.id {
        return None;
    }
    // Turn "claude-sonnet-4-6" into "Claude Sonnet 4 6".
    let words: Vec<String> = leaf
        .split(['-', '_'])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

/// Maps one OmniRoute model into Zed's schema.
pub fn to_zed_model(model: &OmniRouteModel) -> ZedModel {
    let caps = model.capabilities.clone().unwrap_or_default();
    let supports_tools = caps.tool_calling;
    let images = accepts_images(model);

    ZedModel {
        name: model.id.clone(),
        display_name: display_name_for(model),
        max_tokens: model.context_length.unwrap_or(DEFAULT_MAX_TOKENS),
        max_output_tokens: model.max_output_tokens,
        reasoning_effort: default_reasoning_effort(model),
        capabilities: ZedCapabilities {
            // Without tool support Zed would silently drop tool calls, which
            // breaks the Agent panel, so mirror OmniRoute's flag exactly.
            tools: supports_tools,
            // Images mean vision support in Zed's model.
            images,
            // Tools are only sent one at a time unless the backend says so.
            parallel_tool_calls: supports_tools,
            // Prompt-cache keys are an OpenAI extension OmniRoute proxies.
            prompt_cache_key: false,
            // Zed targets the chat-completions wire format, which is what
            // OmniRoute's `/v1` implements.
            chat_completions: true,
            // Interleaved reasoning requires the Responses API shape, which
            // this integration does not use.
            interleaved_reasoning: false,
            // OmniRoute accepts `max_tokens` on chat-completions requests.
            max_tokens_parameter: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_with(json: &str) -> OmniRouteModel {
        serde_json::from_str(json).expect("test model fixture must parse")
    }

    #[test]
    fn maps_minimal_model() {
        let model = model_with(r#"{"id":"auto/cheap"}"#);
        let zed = to_zed_model(&model);
        assert_eq!(zed.name, "auto/cheap");
        assert_eq!(zed.max_tokens, DEFAULT_MAX_TOKENS);
        assert!(zed.capabilities.chat_completions);
        assert!(!zed.capabilities.tools);
        assert!(zed.reasoning_effort.is_none());
    }

    #[test]
    fn maps_context_and_output_tokens() {
        let model = model_with(r#"{"id":"x","context_length":1048576,"max_output_tokens":65536}"#);
        let zed = to_zed_model(&model);
        assert_eq!(zed.max_tokens, 1_048_576);
        assert_eq!(zed.max_output_tokens, Some(65_536));
    }

    #[test]
    fn maps_tool_calling_to_capabilities() {
        let model =
            model_with(r#"{"id":"x","capabilities":{"tool_calling":true,"reasoning":true}}"#);
        let zed = to_zed_model(&model);
        assert!(zed.capabilities.tools);
        assert!(zed.capabilities.parallel_tool_calls);
    }

    #[test]
    fn detects_image_input() {
        let model = model_with(
            r#"{"id":"x","input_modalities":["text","image"],"capabilities":{"tool_calling":true}}"#,
        );
        assert!(to_zed_model(&model).capabilities.images);

        let text_only = model_with(r#"{"id":"y","input_modalities":["text"]}"#);
        assert!(!to_zed_model(&text_only).capabilities.images);
    }

    #[test]
    fn picks_highest_effort_tier() {
        let model = model_with(
            r#"{"id":"x","capabilities":{"reasoning":true,"effort_tiers":["low","medium","high","xhigh","max"]}}"#,
        );
        assert_eq!(
            to_zed_model(&model).reasoning_effort,
            Some(ReasoningEffort::Max)
        );
    }

    #[test]
    fn ignores_unknown_effort_tiers() {
        let model = model_with(
            r#"{"id":"x","capabilities":{"thinking":true,"effort_tiers":["turbo","high"]}}"#,
        );
        assert_eq!(
            to_zed_model(&model).reasoning_effort,
            Some(ReasoningEffort::High)
        );
    }

    #[test]
    fn no_effort_without_reasoning_flag() {
        let model = model_with(r#"{"id":"x","capabilities":{"effort_tiers":["high","max"]}}"#);
        assert!(to_zed_model(&model).reasoning_effort.is_none());
    }

    #[test]
    fn prefers_explicit_display_name() {
        let model = model_with(r#"{"id":"auto/cheap","display_name":"Cheap Model"}"#);
        assert_eq!(
            to_zed_model(&model).display_name.as_deref(),
            Some("Cheap Model")
        );
    }

    #[test]
    fn humanizes_leaf_segment() {
        let model = model_with(r#"{"id":"claude/claude-sonnet-4-6"}"#);
        assert_eq!(
            to_zed_model(&model).display_name.as_deref(),
            Some("Claude Sonnet 4 6")
        );
    }

    #[test]
    fn no_display_name_for_flat_id() {
        let model = model_with(r#"{"id":"gpt-5"}"#);
        assert!(to_zed_model(&model).display_name.is_none());
    }

    #[test]
    fn serializes_to_zed_field_names() {
        let model = model_with(
            r#"{"id":"x","capabilities":{"tool_calling":true,"reasoning":true,"effort_tiers":["high"]}}"#,
        );
        let value = serde_json::to_value(to_zed_model(&model)).unwrap();
        assert_eq!(value["name"], "x");
        assert_eq!(value["reasoning_effort"], "high");
        assert_eq!(value["capabilities"]["tools"], true);
        assert_eq!(value["capabilities"]["chat_completions"], true);
        assert!(value["max_tokens"].is_number());
        // None fields must be omitted, not serialized as null.
        assert!(value.get("display_name").is_none());
        assert!(value.get("max_output_tokens").is_none());
    }

    #[test]
    fn parses_full_envelope() {
        let response: ModelsResponse = serde_json::from_str(
            r#"{"object":"list","data":[{"id":"a"},{"id":"b","context_length":10}]}"#,
        )
        .unwrap();
        assert_eq!(response.data.len(), 2);
        assert_eq!(response.data[1].context_length, Some(10));
    }

    #[test]
    fn tolerates_missing_data_array() {
        let response: ModelsResponse = serde_json::from_str(r#"{"object":"list"}"#).unwrap();
        assert!(response.data.is_empty());
    }

    #[test]
    fn maps_the_real_payload_shape() {
        // Captured verbatim from the live gateway.
        let response: ModelsResponse = serde_json::from_str(
            r#"{"object":"list","data":[{"id":"auto/best-coding","object":"model",
            "created":1791108695,"owned_by":"combo","permission":[],
            "root":"auto/best-coding","parent":null,"context_length":1048576,
            "max_input_tokens":1048576,"max_output_tokens":65536,
            "input_modalities":["text","image"],"output_modalities":["text"],
            "capabilities":{"tool_calling":true,"reasoning":true,"thinking":true,
            "temperature":true}}]}"#,
        )
        .unwrap();
        let zed = to_zed_model(&response.data[0]);
        assert_eq!(zed.name, "auto/best-coding");
        assert_eq!(zed.max_tokens, 1_048_576);
        assert_eq!(zed.max_output_tokens, Some(65_536));
        assert!(zed.capabilities.tools);
        assert!(zed.capabilities.images);
        assert_eq!(zed.display_name.as_deref(), Some("Best Coding"));
    }
}
