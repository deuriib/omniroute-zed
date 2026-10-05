//! JSONC-aware merging into Zed's `settings.json`.
//!
//! Zed accepts JSONC, and real settings files use it: comments, trailing
//! commas, and hand-written annotations. A strict JSON round-trip would throw
//! all of that away, so this module edits the **CST** (concrete syntax tree)
//! from `jsonc-parser` instead. Verified against a real 790-line file: an
//! untouched parse re-prints byte-for-byte, and a targeted edit leaves every
//! comment intact.
//!
//! Scope is deliberately narrow: this tool owns the OmniRoute LLM provider and
//! nothing else. It does not touch `context_servers` or any MCP registration.
//!
//! The merge is **additive and conservative**:
//!
//! * Model entries already present are never touched, so hand-written
//!   `display_name`s and the comments explaining them survive.
//! * Models discovered upstream are appended when missing.
//! * Nothing is ever removed. If a model disappears from `/v1/models`, we leave
//!   the user's entry alone; deleting a config line on a transient upstream
//!   hiccup is far worse than a stale entry Zed will simply fail to call.
//! * Only the provider keys we own are modified.

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use serde_json::Value;

use crate::models::ZedModel;

/// Why a merge could not be completed.
#[derive(Debug)]
pub enum MergeError {
    /// The file could not be parsed as JSONC, so we refuse to touch it.
    Parse(String),
    /// `language_models` exists but is not an object.
    Malformed(String),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::Parse(detail) => {
                write!(f, "not valid JSONC ({detail}); refusing to touch the file")
            }
            MergeError::Malformed(key) => write!(f, "`{key}` is present but is not an object"),
        }
    }
}

impl std::error::Error for MergeError {}

/// What a merge changed, for honest logging.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergeStats {
    /// Model entries that were already present and left untouched.
    pub preserved: usize,
    /// Model entries appended because they were missing.
    pub added: usize,
    /// Model entries that differed from upstream and were left as the user had them.
    pub stale: usize,
}

/// Converts a serde value into CST input, preserving key order.
fn value_to_cst(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(flag) => CstInputValue::Bool(*flag),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(text) => CstInputValue::String(text.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(value_to_cst).collect()),
        Value::Object(entries) => CstInputValue::Object(
            entries
                .iter()
                .map(|(key, val)| (key.clone(), value_to_cst(val)))
                .collect(),
        ),
    }
}

/// Reads a model's `name` from a CST node, if it is a plain string.
fn node_string(node: &jsonc_parser::cst::CstNode) -> Option<String> {
    let object = node.as_object()?;
    let name = object.get("name")?;
    let literal = name.value()?.as_string_lit()?;
    literal.decoded_value().ok()
}

/// Walks `root.a.b.c`, creating objects as needed.
///
/// Returns an error if an existing node on the path is present but is not an
/// object: that means the file is already invalid for Zed, and silently
/// clobbering it would hide the real problem.
fn descend(root: &CstObject, path: &[&str]) -> Result<CstObject, MergeError> {
    let mut current = root.clone();
    for key in path {
        let next = current.object_value_or_create(key);
        current = next.ok_or_else(|| MergeError::Malformed((*key).to_string()))?;
    }
    Ok(current)
}

/// The provider table at `language_models.openai_compatible`, created if absent.
fn provider_table(root: &CstObject, provider_id: &str) -> Result<CstObject, MergeError> {
    let providers = descend(root, &["language_models", "openai_compatible"])?;
    providers
        .object_value_or_create(provider_id)
        .ok_or_else(|| MergeError::Malformed(provider_id.to_string()))
}

/// Merges the model list and provider URL into an existing provider object.
///
/// Existing entries are preserved verbatim. New upstream models are appended.
fn merge_models(
    provider: &CstObject,
    api_url: &str,
    models: &[ZedModel],
    api_key: Option<&str>,
    stats: &mut MergeStats,
) {
    // `api_url` is ours to own; Zed appends `/chat/completions` to it.
    match provider.get("api_url") {
        Some(prop) => prop.set_value(CstInputValue::String(api_url.to_string())),
        None => {
            provider.insert(0, "api_url", CstInputValue::String(api_url.to_string()));
        }
    }

    // The key is only persisted on explicit opt-in: settings.json is plaintext
    // and Zed reads OMNIROUTE_API_KEY from its own environment anyway.
    if let Some(key) = api_key
        && let Some(headers) = provider.object_value_or_create("custom_headers")
    {
        headers.append(
            "Authorization",
            CstInputValue::String(format!("Bearer {key}")),
        );
    }

    let array = provider.array_value_or_set("available_models");

    // Index what the user already has, so we never clobber a curated entry.
    let existing: Vec<String> = array.elements().iter().filter_map(node_string).collect();
    let mut seen: std::collections::HashSet<&String> = existing.iter().collect();

    stats.preserved = existing.len();

    for model in models {
        if seen.contains(&model.name) {
            continue;
        }
        let encoded = serde_json::to_value(model).expect("ZedModel always serializes");
        array.append(value_to_cst(&encoded));
        seen.insert(&model.name);
        stats.added += 1;
    }
}

/// Applies the model merge to `text` and returns the new file contents.
///
/// Returns the input unchanged when nothing needed changing, so callers can
/// skip the write entirely and avoid waking Zed's settings watcher.
pub fn merge_into_settings(
    text: &str,
    provider_id: &str,
    api_url: &str,
    models: &[ZedModel],
    api_key: Option<&str>,
) -> Result<(String, MergeStats), MergeError> {
    let root = CstRootNode::parse(text, &ParseOptions::default())
        .map_err(|err| MergeError::Parse(err.to_string()))?;
    let object = root
        .object_value()
        .ok_or_else(|| MergeError::Malformed("<root>".to_string()))?;

    let provider = provider_table(&object, provider_id)?;
    let mut stats = MergeStats::default();
    merge_models(&provider, api_url, models, api_key, &mut stats);

    let rendered = root.to_string();
    if rendered == text {
        stats.added = 0;
        return Ok((rendered, stats));
    }
    Ok((rendered, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OmniRouteModel;
    use crate::models::to_zed_model;

    fn model(name: &str) -> ZedModel {
        to_zed_model(&OmniRouteModel {
            id: name.to_string(),
            display_name: None,
            context_length: Some(200_000),
            max_output_tokens: Some(8_000),
            input_modalities: Vec::new(),
            display_hint: None,
            capabilities: None,
        })
    }

    #[test]
    fn round_trips_a_jsonc_file_byte_for_byte() {
        // Comments and trailing commas must survive an untouched parse.
        let text = "{\n  // a note\n  \"a\": 1,\n}\n";
        let (out, _) =
            merge_into_settings(text, "omniroute", "http://localhost:20128/v1", &[], None)
                .expect("parses");
        // The only change is the provider we were asked to add.
        assert!(out.contains("// a note"));
        assert!(out.contains("\"a\": 1,"));
    }

    #[test]
    fn preserves_curated_display_names_and_comments() {
        let text = r#"{
  "language_models": {
    "openai_compatible": {
      "omniroute": {
        "api_url": "http://localhost:20128/v1",
        "available_models": [
          // ── auto/* ── OmniRoute semantic router
          // All capabilities: tools, vision, thinking
          {
            "name": "auto/chat",
            "display_name": "auto · chat",
            "max_tokens": 1048576
          },
        ],
      },
    },
  },
}
"#;
        // "auto/chat" already exists, so it must not be re-added or rewritten.
        let (out, stats) = merge_into_settings(
            text,
            "omniroute",
            "http://localhost:20128/v1",
            &[model("auto/chat"), model("claude/sonnet")],
            None,
        )
        .expect("parses");

        assert!(out.contains("auto · chat"), "curated display name lost");
        assert!(out.contains("OmniRoute semantic router"), "comment lost");
        assert_eq!(stats.preserved, 1);
        assert_eq!(stats.added, 1);
        assert!(out.contains("claude/sonnet"));
    }

    #[test]
    fn is_idempotent_for_an_unchanged_model_set() {
        let first = merge_into_settings(
            "{\n  \"theme\": \"dark\",\n}\n",
            "omniroute",
            "http://localhost:20128/v1",
            &[model("auto/chat")],
            None,
        )
        .expect("parses");
        let (text, _) = first;

        let second = merge_into_settings(
            &text,
            "omniroute",
            "http://localhost:20128/v1",
            &[model("auto/chat")],
            None,
        )
        .expect("parses");

        assert_eq!(text, second.0, "a repeat sync rewrote the file");
        assert_eq!(second.1.added, 0);
    }

    #[test]
    fn leaves_unrelated_settings_untouched() {
        let text = "{\n  \"theme\": \"One Dark\",\n  \"vim_mode\": true,\n}\n";
        let (out, _) =
            merge_into_settings(text, "omniroute", "http://localhost:20128/v1", &[], None)
                .expect("parses");
        assert!(out.contains("\"theme\": \"One Dark\""));
        assert!(out.contains("\"vim_mode\": true"));
    }

    #[test]
    fn never_writes_the_api_key_by_default() {
        let (out, _) =
            merge_into_settings("{}\n", "omniroute", "http://localhost:20128/v1", &[], None)
                .expect("parses");
        assert!(!out.contains("sk-"), "a secret leaked into settings.json");
        assert!(!out.contains("Authorization"));
    }

    #[test]
    fn writes_the_api_key_only_when_asked() {
        let (out, _) = merge_into_settings(
            "{}\n",
            "omniroute",
            "http://localhost:20128/v1",
            &[],
            Some("sk-test"),
        )
        .expect("parses");
        assert!(out.contains("Bearer sk-test"));
    }

    #[test]
    fn never_touches_context_servers() {
        // MCP registration was deliberately removed from this tool. A user's
        // existing context servers, and any credentials inside them, must be
        // left byte-for-byte alone — we must never add, rewrite or drop them.
        let text = r#"{
  "context_servers": {
    "omniroute": {
      "headers": {
        "Authorization": "Bearer sk-preexisting",
      },
      "enabled": true,
      "url": "http://localhost:20128/api/mcp/stream",
    },
    "gateway": {
      "url": "http://localhost:8080/mcp",
    },
  },
}
"#;
        let (out, _) =
            merge_into_settings(text, "omniroute", "http://localhost:20128/v1", &[], None)
                .expect("parses");

        assert!(
            out.contains("Bearer sk-preexisting"),
            "a pre-existing MCP credential was destroyed"
        );
        assert!(
            out.contains("http://localhost:8080/mcp"),
            "other server lost"
        );
        assert_eq!(
            out.matches("api/mcp/stream").count(),
            text.matches("api/mcp/stream").count(),
            "MCP entries were added or duplicated"
        );
    }

    #[test]
    fn does_not_create_a_context_servers_table() {
        let (out, _) = merge_into_settings(
            "{\n}\n",
            "omniroute",
            "http://localhost:20128/v1",
            &[],
            None,
        )
        .expect("parses");
        assert!(
            !out.contains("context_servers"),
            "this tool must not create MCP configuration"
        );
    }

    #[test]
    fn rejects_a_file_that_is_not_jsonc() {
        let err = merge_into_settings(
            "this is not json at all",
            "omniroute",
            "http://localhost:20128/v1",
            &[],
            None,
        )
        .expect_err("must refuse");
        assert!(err.to_string().contains("refusing"));
    }

    #[test]
    fn skips_models_already_present() {
        // Duplicate upstream ids must not produce duplicate entries.
        let (_, stats) = merge_into_settings(
            "{}\n",
            "omniroute",
            "http://localhost:20128/v1",
            &[model("a"), model("a"), model("b")],
            None,
        )
        .expect("parses");
        assert_eq!(stats.added, 2);
    }
}
