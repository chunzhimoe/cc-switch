//! Global third-party model-list passthrough and public model-id rewriting.

use crate::app_config::AppType;
use crate::database::Database;
use crate::provider::{ModelListProxyConfig, Provider};
use crate::proxy::error::ProxyError;
use crate::proxy::providers::{get_adapter, AuthStrategy, ProviderAdapter};
use reqwest::header::USER_AGENT;
use reqwest::StatusCode;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE_TTL: Duration = Duration::from_secs(45);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const ERROR_BODY_MAX_CHARS: usize = 512;

#[derive(Clone)]
pub struct GlobalModelSource {
    pub app_type: AppType,
    pub provider: Provider,
    pub config: ModelListProxyConfig,
}

#[derive(Clone)]
struct CachedModels {
    key: String,
    fetched_at: Instant,
    response: Value,
    public_to_upstream: HashMap<String, String>,
}

#[derive(Clone)]
pub struct GlobalModelSnapshot {
    pub response: Value,
}

fn cache() -> &'static Mutex<Option<CachedModels>> {
    static CACHE: OnceLock<Mutex<Option<CachedModels>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

pub fn find_global_source(db: &Database) -> Result<Option<GlobalModelSource>, ProxyError> {
    for app_type in AppType::all() {
        if matches!(app_type, AppType::ClaudeDesktop | AppType::GrokBuild) {
            continue;
        }
        let providers = db
            .get_all_providers(app_type.as_str())
            .map_err(|err| ProxyError::DatabaseError(err.to_string()))?;
        for provider in providers.into_values() {
            let Some(config) = provider
                .meta
                .as_ref()
                .and_then(|meta| meta.model_list_proxy.as_ref())
                .filter(|config| config.is_global_source)
                .cloned()
            else {
                continue;
            };
            return Ok(Some(GlobalModelSource {
                app_type,
                provider,
                config,
            }));
        }
    }
    Ok(None)
}

pub async fn fetch_global_models(db: &Database) -> Result<Option<Value>, ProxyError> {
    Ok(fetch_global_snapshot(db).await?.map(|snapshot| snapshot.response))
}

pub async fn fetch_global_snapshot(
    db: &Database,
) -> Result<Option<GlobalModelSnapshot>, ProxyError> {
    let Some(source) = find_global_source(db)? else {
        return Ok(None);
    };
    let key = source_cache_key(&source)?;

    if let Some(cached) = cache()
        .lock()
        .expect("global model cache poisoned")
        .as_ref()
        .filter(|cached| cached.key == key && cached.fetched_at.elapsed() < CACHE_TTL)
        .cloned()
    {
        return Ok(Some(GlobalModelSnapshot {
            response: cached.response,
        }));
    }

    let (response, public_to_upstream) = fetch_and_transform(&source).await?;
    *cache().lock().expect("global model cache poisoned") = Some(CachedModels {
        key,
        fetched_at: Instant::now(),
        response: response.clone(),
        public_to_upstream: public_to_upstream.clone(),
    });
    Ok(Some(GlobalModelSnapshot { response }))
}

pub async fn restore_body_model_with_refresh(
    db: &Database,
    body: &mut Value,
) -> Result<bool, ProxyError> {
    if !ensure_active_cache(db).await? {
        return Ok(false);
    }
    Ok(restore_body_model(body))
}

pub async fn restore_gemini_endpoint_with_refresh(
    db: &Database,
    endpoint: &str,
) -> Result<Option<String>, ProxyError> {
    if !ensure_active_cache(db).await? {
        return Ok(None);
    }
    Ok(restore_gemini_endpoint(endpoint))
}

async fn ensure_active_cache(db: &Database) -> Result<bool, ProxyError> {
    let Some(source) = find_global_source(db)? else {
        *cache().lock().expect("global model cache poisoned") = None;
        return Ok(false);
    };
    let active_key = source_cache_key(&source)?;
    let cache_is_current = cache()
        .lock()
        .expect("global model cache poisoned")
        .as_ref()
        .is_some_and(|cached| {
            cached.key == active_key && cached.fetched_at.elapsed() < CACHE_TTL
        });
    if !cache_is_current {
        fetch_global_snapshot(db).await?;
    }
    Ok(true)
}

pub fn restore_public_model_name(model: &str) -> Option<String> {
    let (base, suffix) = split_one_m_suffix(model);
    let cache = cache().lock().expect("global model cache poisoned");
    let upstream = cache.as_ref()?.public_to_upstream.get(base)?;
    Some(format!("{upstream}{suffix}"))
}

pub fn is_known_upstream_model(model: &str) -> bool {
    let (base, _) = split_one_m_suffix(model);
    cache()
        .lock()
        .expect("global model cache poisoned")
        .as_ref()
        .is_some_and(|cached| cached.public_to_upstream.values().any(|value| value == base))
}

pub fn restore_body_model(body: &mut Value) -> bool {
    let Some(model) = body.get("model").and_then(Value::as_str) else {
        return false;
    };
    let (base, _) = split_one_m_suffix(model);
    let cache = cache().lock().expect("global model cache poisoned");
    let Some(cached) = cache.as_ref() else {
        return false;
    };
    let known_upstream = cached
        .public_to_upstream
        .values()
        .any(|value| value == base);
    let upstream = cached.public_to_upstream.get(base).cloned();
    drop(cache);

    let Some(upstream) = upstream else {
        return known_upstream;
    };
    let (_, suffix) = split_one_m_suffix(model);
    let restored = format!("{upstream}{suffix}");
    if restored != model {
        log::debug!("[GlobalModels] restoring public model: {model} -> {restored}");
        body["model"] = Value::String(restored);
    }
    true
}

pub fn restore_gemini_endpoint(endpoint: &str) -> Option<String> {
    let marker = "/models/";
    let start = endpoint.find(marker)? + marker.len();
    let tail = &endpoint[start..];
    let model_end = tail.find([':', '?']).unwrap_or(tail.len());
    let encoded_model = &tail[..model_end];
    let decoded_model = percent_decode_model_id(encoded_model);
    let upstream = restore_public_model_name(&decoded_model)?;
    if upstream == decoded_model {
        return None;
    }
    let mut restored = endpoint.to_string();
    let encoded_upstream = percent_encode_model_id(&upstream);
    restored.replace_range(start..start + model_end, &encoded_upstream);
    Some(restored)
}

fn percent_encode_model_id(value: &str) -> String {
    value.replace('%', "%25").replace('/', "%2F")
}

fn percent_decode_model_id(value: &str) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%' && index + 2 < raw.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(decoded) = u8::from_str_radix(hex, 16) {
                bytes.push(decoded);
                index += 3;
                continue;
            }
        }
        bytes.push(raw[index]);
        index += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| value.to_string())
}

fn source_cache_key(source: &GlobalModelSource) -> Result<String, ProxyError> {
    let adapter = get_adapter(&source.app_type);
    let base_url = adapter.extract_base_url(&source.provider)?;
    Ok(format!(
        "{}:{}:{}:{}:{}",
        source.app_type.as_str(),
        source.provider.id,
        base_url,
        source.config.models_url.as_deref().unwrap_or_default(),
        source.config.strip_prefix.as_deref().unwrap_or_default()
    ))
}

async fn fetch_and_transform(
    source: &GlobalModelSource,
) -> Result<(Value, HashMap<String, String>), ProxyError> {
    let adapter = get_adapter(&source.app_type);
    let base_url = adapter.extract_base_url(&source.provider)?;
    let is_full_url = source
        .provider
        .meta
        .as_ref()
        .and_then(|meta| meta.is_full_url)
        .unwrap_or(false);
    let candidates = crate::services::model_fetch::build_models_url_candidates(
        &base_url,
        is_full_url,
        source.config.models_url.as_deref(),
    )
    .map_err(ProxyError::ConfigError)?;
    // Managed-account OAuth providers need live token managers that are not
    // available to this lightweight list fetch. Reject them explicitly rather
    // than sending adapter placeholders as credentials.
    if source.provider.is_github_copilot()
        || source.provider.is_codex_oauth()
        || source.provider.is_xai_oauth()
    {
        return Err(ProxyError::ConfigError(
            "managed OAuth providers cannot be used as the global models source yet".to_string(),
        ));
    }
    let auth = adapter
        .extract_auth(&source.provider)
        .ok_or_else(|| ProxyError::AuthError("model-list source has no credentials".to_string()))?;
    let auth_headers = adapter.get_auth_headers(&auth)?;
    let user_agent = source
        .provider
        .meta
        .as_ref()
        .map(|meta| meta.custom_user_agent_header())
        .transpose()
        .map_err(|err| ProxyError::ConfigError(format!("invalid custom User-Agent: {err}")))?
        .flatten();

    let mut last_error = None;
    for url in candidates {
        log::debug!("[GlobalModels] fetching {}", crate::redact_url_for_log(&url));
        let mut request = crate::proxy::http_client::get()
            .get(&url)
            .timeout(FETCH_TIMEOUT);
        // Most third-party `/models` endpoints use OpenAI-compatible Bearer
        // authentication even when their message endpoint accepts x-api-key.
        // Preserve Google/OAuth-specific headers; normalize plain static keys
        // to Bearer to match the existing provider-form model fetch behavior.
        if matches!(auth.strategy, AuthStrategy::Google | AuthStrategy::GoogleOAuth) {
            for (name, value) in &auth_headers {
                request = request.header(name, value);
            }
        } else {
            request = request.header("Authorization", format!("Bearer {}", auth.api_key));
        }
        if let Some(value) = user_agent.as_ref() {
            request = request.header(USER_AGENT, value);
        }

        let response = request
            .send()
            .await
            .map_err(|err| ProxyError::ForwardFailed(format!("models request failed: {err}")))?;
        let status = response.status();
        if status.is_success() {
            let mut value: Value = response.json().await.map_err(|err| {
                ProxyError::TransformError(format!("failed to parse models response: {err}"))
            })?;
            let mut mapping = HashMap::new();
            transform_models_response(
                &mut value,
                source.config.strip_prefix.as_deref().unwrap_or_default(),
                &mut mapping,
            );
            return Ok((value, mapping));
        }

        let body = truncate_body(response.text().await.unwrap_or_default());
        if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
            last_error = Some(format!("HTTP {status}: {body}"));
            continue;
        }
        return Err(ProxyError::UpstreamError {
            status: status.as_u16(),
            body: Some(body),
        });
    }

    Err(ProxyError::ForwardFailed(format!(
        "all models endpoints failed: {}",
        last_error.unwrap_or_else(|| "no endpoint candidates".to_string())
    )))
}

fn transform_models_response(
    response: &mut Value,
    prefix: &str,
    mapping: &mut HashMap<String, String>,
) {
    if prefix.is_empty() {
        collect_model_mappings(response, prefix, mapping);
        return;
    }

    if let Some(items) = response.get_mut("data").and_then(Value::as_array_mut) {
        for item in items {
            transform_model_object(item, prefix, mapping);
        }
        return;
    }
    if let Some(items) = response.get_mut("models").and_then(Value::as_array_mut) {
        for item in items {
            transform_model_object(item, prefix, mapping);
        }
        return;
    }
    transform_model_object(response, prefix, mapping);
}

fn collect_model_mappings(response: &Value, prefix: &str, mapping: &mut HashMap<String, String>) {
    if let Some(items) = response.get("data").and_then(Value::as_array) {
        for item in items {
            collect_model_mapping(item, prefix, mapping);
        }
    } else if let Some(items) = response.get("models").and_then(Value::as_array) {
        for item in items {
            collect_model_mapping(item, prefix, mapping);
        }
    } else {
        collect_model_mapping(response, prefix, mapping);
    }
}

fn transform_model_object(value: &mut Value, prefix: &str, mapping: &mut HashMap<String, String>) {
    let Some(object) = value.as_object_mut() else {
        return;
    };

    if let Some(original) = object.get("id").and_then(Value::as_str).map(str::to_string) {
        let public = original.strip_prefix(prefix).unwrap_or(&original).to_string();
        mapping.insert(public.clone(), original);
        object.insert("id".to_string(), Value::String(public));
    }

    for key in ["real_id", "display_name"] {
        let Some(original) = object.get(key).and_then(Value::as_str) else {
            continue;
        };
        let public = original.strip_prefix(prefix).unwrap_or(original).to_string();
        object.insert(key.to_string(), Value::String(public));
    }
}

fn collect_model_mapping(value: &Value, prefix: &str, mapping: &mut HashMap<String, String>) {
    let Some(original) = value.get("id").and_then(Value::as_str) else {
        return;
    };
    let public = original.strip_prefix(prefix).unwrap_or(original);
    mapping.insert(public.to_string(), original.to_string());
}

fn split_one_m_suffix(model: &str) -> (&str, &str) {
    let trimmed = model.trim_end();
    for suffix in ["[1m]", "[1M]"] {
        if let Some(base) = trimmed.strip_suffix(suffix) {
            let base = base.trim_end();
            return (base, &trimmed[base.len()..]);
        }
    }
    (model, "")
}

fn truncate_body(body: String) -> String {
    if body.chars().count() <= ERROR_BODY_MAX_CHARS {
        body
    } else {
        let mut value: String = body.chars().take(ERROR_BODY_MAX_CHARS).collect();
        value.push('…');
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rewrites_only_model_identity_fields() {
        let mut response = json!({
            "object": "list",
            "request_id": "claude-keep",
            "data": [{
                "id": "claude-qd/auto",
                "real_id": "claude-qd/auto",
                "display_name": "claude-qd/auto",
                "capabilities": {"id": "claude-capability", "vision": true}
            }]
        });
        let mut mapping = HashMap::new();
        transform_models_response(&mut response, "claude-", &mut mapping);
        assert_eq!(response["request_id"], "claude-keep");
        assert_eq!(response["data"][0]["id"], "qd/auto");
        assert_eq!(response["data"][0]["real_id"], "qd/auto");
        assert_eq!(response["data"][0]["display_name"], "qd/auto");
        assert_eq!(response["data"][0]["capabilities"]["id"], "claude-capability");
        assert_eq!(mapping.get("qd/auto").map(String::as_str), Some("claude-qd/auto"));
    }

    #[test]
    fn rewrites_models_collection_without_losing_fields() {
        let mut response = json!({
            "models": [{"id": "vendor-a", "context_window": 200000}],
            "next": "token"
        });
        let mut mapping = HashMap::new();
        transform_models_response(&mut response, "vendor-", &mut mapping);
        assert_eq!(response["models"][0]["id"], "a");
        assert_eq!(response["models"][0]["context_window"], 200000);
        assert_eq!(response["next"], "token");
    }

    #[test]
    fn restores_slash_model_from_percent_encoded_gemini_path() {
        *cache().lock().unwrap() = Some(CachedModels {
            key: "test".to_string(),
            fetched_at: Instant::now(),
            response: json!({}),
            public_to_upstream: HashMap::from([(
                "qd/auto".to_string(),
                "claude-qd/auto".to_string(),
            )]),
        });
        assert_eq!(
            restore_gemini_endpoint("/v1beta/models/qd%2Fauto:generateContent?key=x"),
            Some("/v1beta/models/claude-qd%2Fauto:generateContent?key=x".to_string())
        );
    }
}
