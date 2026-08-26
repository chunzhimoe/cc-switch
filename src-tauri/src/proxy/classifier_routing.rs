//! Provider-scoped routing for Claude Code's Auto safety classifier.

use crate::provider::ClassifierRoutingConfig;
use serde_json::Value;

pub const CLASSIFIER_SYSTEM_MARKER: &str =
    "You are a security monitor for autonomous AI coding agents.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifierRoute {
    pub before_model: Option<String>,
    pub model: String,
    pub log_hits: bool,
}

fn block_text(block: &Value) -> Option<&str> {
    if let Some(text) = block.as_str() {
        return Some(text);
    }

    if block.get("type").and_then(Value::as_str) == Some("text") {
        return block.get("text").and_then(Value::as_str);
    }

    None
}

/// Match the Claudish behavior without allocating a trimmed copy of the system block.
fn starts_with_marker(text: &str) -> bool {
    let mut index = 0;
    while index < text.len() {
        let ch = text.as_bytes()[index];
        if !matches!(ch, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b) {
            break;
        }
        index += 1;
    }

    text.get(index..)
        .is_some_and(|candidate| candidate.starts_with(CLASSIFIER_SYSTEM_MARKER))
}

/// Return true when any Claude Messages system text block starts with the marker.
pub fn is_auto_mode_classifier_request(body: &Value) -> bool {
    let Some(system) = body.get("system") else {
        return false;
    };

    if let Some(text) = system.as_str() {
        return starts_with_marker(text);
    }

    let Some(blocks) = system.as_array() else {
        return false;
    };

    blocks
        .iter()
        .filter_map(block_text)
        .any(starts_with_marker)
}

fn valid_model_id(value: &str) -> Option<String> {
    let model = value.trim();
    (!model.is_empty()).then(|| model.to_string())
}

fn first_model(config: &ClassifierRoutingConfig) -> Option<String> {
    config
        .models
        .iter()
        .find_map(|entry| valid_model_id(&entry.id))
}

fn default_model(config: &ClassifierRoutingConfig) -> Option<String> {
    valid_model_id(&config.default_model).or_else(|| first_model(config))
}

fn cheapest_model(config: &ClassifierRoutingConfig) -> Option<String> {
    config
        .models
        .iter()
        .filter_map(|entry| {
            let input = entry.input_price?;
            let output = entry.output_price?;
            if !input.is_finite() || !output.is_finite() || input < 0.0 || output < 0.0 {
                return None;
            }
            Some((input + output, valid_model_id(&entry.id)?))
        })
        .min_by(|(price_a, _), (price_b, _)| {
            price_a
                .partial_cmp(price_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, model)| model)
        .or_else(|| default_model(config))
}

/// Select a configured classifier model without ever returning an empty id.
pub fn pick_classifier_model(config: &ClassifierRoutingConfig) -> Option<String> {
    match config.strategy.trim().to_ascii_lowercase().as_str() {
        "fixed" => default_model(config),
        "cheapest" => cheapest_model(config),
        // priority_list and unknown values both use the safe list-first behavior.
        _ => first_model(config).or_else(|| default_model(config)),
    }
}

/// Resolve a route from the original, pre-transform Anthropic request body.
pub fn resolve_classifier_route(
    body: &Value,
    config: &ClassifierRoutingConfig,
) -> Option<ClassifierRoute> {
    if !config.enabled || !is_auto_mode_classifier_request(body) {
        return None;
    }

    Some(ClassifierRoute {
        before_model: body
            .get("model")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        model: pick_classifier_model(config)?,
        log_hits: config.log_hits,
    })
}

/// Apply the classifier model and remove thinking, matching Claudish's rewrite.
pub fn enforce_classifier_route(body: &mut Value, route: &ClassifierRoute) {
    let Some(object) = body.as_object_mut() else {
        return;
    };

    object.insert("model".to_string(), Value::String(route.model.clone()));
    object.remove("thinking");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ClassifierModelEntry, ClassifierRoutingConfig};
    use serde_json::json;

    fn config() -> ClassifierRoutingConfig {
        ClassifierRoutingConfig {
            enabled: true,
            strategy: "priority_list".to_string(),
            default_model: "default-model".to_string(),
            models: vec![
                ClassifierModelEntry {
                    id: "first-model".to_string(),
                    note: None,
                    input_price: Some(2.0),
                    output_price: Some(4.0),
                },
                ClassifierModelEntry {
                    id: "cheap-model".to_string(),
                    note: None,
                    input_price: Some(0.1),
                    output_price: Some(0.2),
                },
            ],
            log_hits: true,
        }
    }

    fn classifier_body() -> Value {
        json!({
            "model": "claude-sonnet-5",
            "thinking": {"type": "enabled", "budget_tokens": 1024},
            "system": [
                {"type": "text", "text": "x-anthropic-billing-header: opaque"},
                {"type": "text", "text": format!("  \n{} Evaluate this tool call.", CLASSIFIER_SYSTEM_MARKER)}
            ],
            "temperature": 0
        })
    }

    #[test]
    fn detects_marker_in_later_text_block() {
        assert!(is_auto_mode_classifier_request(&classifier_body()));
    }

    #[test]
    fn detects_string_system_and_ascii_leading_whitespace() {
        let body = json!({
            "system": format!("\t\n{} Continue.", CLASSIFIER_SYSTEM_MARKER)
        });
        assert!(is_auto_mode_classifier_request(&body));
    }

    #[test]
    fn rejects_main_prompt_and_mid_block_marker() {
        assert!(!is_auto_mode_classifier_request(&json!({
            "system": "You are Claude Code, Anthropic's official CLI for Claude."
        })));
        assert!(!is_auto_mode_classifier_request(&json!({
            "system": format!("Preamble {}", CLASSIFIER_SYSTEM_MARKER)
        })));
    }

    #[test]
    fn rejects_missing_or_malformed_system() {
        assert!(!is_auto_mode_classifier_request(&json!({})));
        assert!(!is_auto_mode_classifier_request(&json!({"system": []})));
        assert!(!is_auto_mode_classifier_request(&json!({
            "system": [{"type": "image", "source": {}}]
        })));
        assert!(!is_auto_mode_classifier_request(&json!({"system": 42})));
    }

    #[test]
    fn picks_each_strategy_and_falls_back_safely() {
        let mut fixed = config();
        fixed.strategy = "fixed".to_string();
        assert_eq!(pick_classifier_model(&fixed).as_deref(), Some("default-model"));

        assert_eq!(pick_classifier_model(&config()).as_deref(), Some("first-model"));

        let mut cheapest = config();
        cheapest.strategy = "cheapest".to_string();
        assert_eq!(pick_classifier_model(&cheapest).as_deref(), Some("cheap-model"));

        let mut unknown = config();
        unknown.strategy = "future_strategy".to_string();
        assert_eq!(pick_classifier_model(&unknown).as_deref(), Some("first-model"));

        let mut no_prices = cheapest.clone();
        no_prices.models.iter_mut().for_each(|entry| {
            entry.input_price = None;
            entry.output_price = None;
        });
        assert_eq!(pick_classifier_model(&no_prices).as_deref(), Some("default-model"));
    }

    #[test]
    fn disabled_or_unconfigured_routing_is_a_noop() {
        let mut disabled = config();
        disabled.enabled = false;
        assert!(resolve_classifier_route(&classifier_body(), &disabled).is_none());

        let mut empty = config();
        empty.default_model.clear();
        empty.models.clear();
        assert!(resolve_classifier_route(&classifier_body(), &empty).is_none());
    }

    #[test]
    fn final_enforcement_wins_over_generic_body_overrides() {
        let mut body = classifier_body();
        let route = resolve_classifier_route(&body, &config()).expect("classifier route");
        enforce_classifier_route(&mut body, &route);

        body["model"] = json!("generic-override");
        body["thinking"] = json!({"type": "enabled"});
        enforce_classifier_route(&mut body, &route);

        assert_eq!(body["model"], "first-model");
        assert!(body.get("thinking").is_none());
    }

        let mut body = classifier_body();
        let route = resolve_classifier_route(&body, &config()).expect("classifier route");
        enforce_classifier_route(&mut body, &route);

        assert_eq!(body["model"], "first-model");
        assert!(body.get("thinking").is_none());
        assert_eq!(body["temperature"], 0);
        assert!(body.get("system").is_some());
        assert_eq!(route.before_model.as_deref(), Some("claude-sonnet-5"));
        assert!(route.log_hits);
    }
}
