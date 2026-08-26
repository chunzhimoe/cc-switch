//! Windsurf MCP sync and import module.
//!
//! Windsurf stores user MCP servers in `%APPDATA%/devin/mcp_config.json`
//! (or the platform equivalent returned by `dirs::config_dir()`). The live
//! format uses a top-level `mcpServers` map. Remote servers use `serverUrl`
//! instead of the unified CC Switch `url` field.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::app_config::{McpApps, McpServer, MultiAppConfig};
use crate::config::{read_json_file, write_json_file};
use crate::error::AppError;

use super::validation::validate_server_spec;

const MCP_SERVERS_KEY: &str = "mcpServers";
const CORE_FIELDS: &[&str] = &[
    "type",
    "command",
    "args",
    "env",
    "cwd",
    "url",
    "serverUrl",
    "headers",
];

pub fn get_windsurf_mcp_config_path() -> Result<PathBuf, AppError> {
    dirs::config_dir()
        .map(|dir| dir.join("devin").join("mcp_config.json"))
        .ok_or_else(|| {
            AppError::localized(
                "config_dir_not_found",
                "无法确定 Windsurf MCP 配置目录",
                "Cannot determine the Windsurf MCP configuration directory",
            )
        })
}

fn should_sync_windsurf_mcp(path: &std::path::Path) -> bool {
    path.exists() || path.parent().is_some_and(std::path::Path::exists)
}

fn read_document(path: &std::path::Path) -> Result<Value, AppError> {
    if !path.exists() {
        return Ok(json!({ MCP_SERVERS_KEY: {} }));
    }

    let value: Value = read_json_file(path)?;
    if !value.is_object() {
        return Err(AppError::Config(format!(
            "Windsurf MCP config must be a JSON object: {}",
            path.display()
        )));
    }
    Ok(value)
}

fn mcp_servers_mut(document: &mut Value) -> Result<&mut Map<String, Value>, AppError> {
    let root = document.as_object_mut().ok_or_else(|| {
        AppError::Config("Windsurf MCP config root must be a JSON object".to_string())
    })?;
    let servers = root
        .entry(MCP_SERVERS_KEY.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    servers.as_object_mut().ok_or_else(|| {
        AppError::Config("Windsurf mcpServers must be a JSON object".to_string())
    })
}

fn convert_to_windsurf_format(spec: &Value) -> Result<Value, AppError> {
    validate_server_spec(spec)?;
    let obj = spec
        .as_object()
        .ok_or_else(|| AppError::McpValidation("MCP spec must be a JSON object".into()))?;
    let kind = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_else(|| if obj.contains_key("command") { "stdio" } else { "http" });

    let mut result = Map::new();
    match kind {
        "stdio" => {
            for field in ["command", "args", "env", "cwd"] {
                if let Some(value) = obj.get(field) {
                    result.insert(field.to_string(), value.clone());
                }
            }
        }
        "http" | "sse" => {
            let url = obj
                .get("url")
                .or_else(|| obj.get("serverUrl"))
                .ok_or_else(|| {
                    AppError::McpValidation("Remote MCP server requires a url".into())
                })?;
            result.insert("serverUrl".to_string(), url.clone());
            if let Some(headers) = obj.get("headers") {
                result.insert("headers".to_string(), headers.clone());
            }
        }
        other => {
            return Err(AppError::McpValidation(format!(
                "Unsupported Windsurf MCP type: {other}"
            )))
        }
    }
    Ok(Value::Object(result))
}

fn convert_from_windsurf_format(id: &str, spec: &Value) -> Result<Value, AppError> {
    let obj = spec.as_object().ok_or_else(|| {
        AppError::McpValidation(format!(
            "Windsurf MCP server '{id}' must be a JSON object"
        ))
    })?;
    let mut result = Map::new();

    if obj.contains_key("command") {
        result.insert("type".to_string(), json!("stdio"));
        for field in ["command", "args", "env", "cwd"] {
            if let Some(value) = obj.get(field) {
                result.insert(field.to_string(), value.clone());
            }
        }
    } else if let Some(url) = obj.get("serverUrl").or_else(|| obj.get("url")) {
        result.insert("type".to_string(), json!("http"));
        result.insert("url".to_string(), url.clone());
        if let Some(headers) = obj.get("headers") {
            result.insert("headers".to_string(), headers.clone());
        }
    } else {
        return Err(AppError::McpValidation(format!(
            "Windsurf MCP server '{id}' has neither command nor serverUrl"
        )));
    }

    let result = Value::Object(result);
    validate_server_spec(&result)?;
    Ok(result)
}

fn merge_windsurf_spec(existing: Option<&Value>, replacement: &Value) -> Value {
    let mut merged = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for field in CORE_FIELDS {
        merged.remove(*field);
    }
    if let Some(replacement) = replacement.as_object() {
        merged.extend(replacement.clone());
    }
    Value::Object(merged)
}

pub fn sync_single_server_to_windsurf(
    _config: &MultiAppConfig,
    id: &str,
    server_spec: &Value,
) -> Result<(), AppError> {
    let path = get_windsurf_mcp_config_path()?;
    if !should_sync_windsurf_mcp(&path) {
        return Ok(());
    }

    let replacement = convert_to_windsurf_format(server_spec)?;
    let mut document = read_document(&path)?;
    let servers = mcp_servers_mut(&mut document)?;
    let merged = merge_windsurf_spec(servers.get(id), &replacement);
    servers.insert(id.to_string(), merged);
    write_json_file(&path, &document)
}

pub fn remove_server_from_windsurf(id: &str) -> Result<(), AppError> {
    let path = get_windsurf_mcp_config_path()?;
    if !path.exists() {
        return Ok(());
    }

    let mut document = read_document(&path)?;
    mcp_servers_mut(&mut document)?.remove(id);
    write_json_file(&path, &document)
}

pub fn import_from_windsurf(config: &mut MultiAppConfig) -> Result<usize, AppError> {
    let path = get_windsurf_mcp_config_path()?;
    if !path.exists() {
        return Ok(0);
    }

    let document = read_document(&path)?;
    let live_servers = document
        .get(MCP_SERVERS_KEY)
        .and_then(Value::as_object)
        .ok_or_else(|| AppError::Config("Windsurf mcpServers must be a JSON object".to_string()))?;
    let servers = config.mcp.servers.get_or_insert_with(HashMap::new);
    let mut changed = 0;

    for (id, spec) in live_servers {
        let unified = match convert_from_windsurf_format(id, spec) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("Skip invalid Windsurf MCP server '{id}': {error}");
                continue;
            }
        };

        if let Some(existing) = servers.get_mut(id) {
            if !existing.apps.windsurf {
                existing.apps.windsurf = true;
                changed += 1;
            }
        } else {
            servers.insert(
                id.clone(),
                McpServer {
                    id: id.clone(),
                    name: id.clone(),
                    server: unified,
                    apps: McpApps {
                        windsurf: true,
                        ..Default::default()
                    },
                    description: None,
                    homepage: None,
                    docs: None,
                    tags: Vec::new(),
                },
            );
            changed += 1;
        }
    }

    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_windsurf_remote_server_url() {
        let unified = convert_from_windsurf_format(
            "remote",
            &json!({"serverUrl":"https://example.com/mcp","headers":{"X-Test":"1"}}),
        )
        .expect("convert remote");
        assert_eq!(unified["type"], "http");
        assert_eq!(unified["url"], "https://example.com/mcp");

        let live = convert_to_windsurf_format(&unified).expect("convert to live");
        assert_eq!(live["serverUrl"], "https://example.com/mcp");
        assert!(live.get("url").is_none());
    }

    #[test]
    fn preserves_windsurf_specific_fields_when_updating() {
        let existing = json!({
            "serverUrl": "https://old.example/mcp",
            "disabled": true,
            "custom": {"keep": true}
        });
        let replacement = json!({"serverUrl":"https://new.example/mcp"});
        let merged = merge_windsurf_spec(Some(&existing), &replacement);

        assert_eq!(merged["serverUrl"], "https://new.example/mcp");
        assert_eq!(merged["disabled"], true);
        assert_eq!(merged["custom"]["keep"], true);
    }
}
