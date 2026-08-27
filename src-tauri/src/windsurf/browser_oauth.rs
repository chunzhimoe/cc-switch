//! Windsurf browser OAuth (wait-callback) login.
//!
//! Flow mirrors cockpit-tools:
//! 1. Start a local HTTP listener on `127.0.0.1:{port}/windsurf-auth-callback`
//! 2. Open the Windsurf sign-in page with `redirect_uri` pointing at that listener
//! 3. Wait for `access_token` (Firebase id token) via automatic redirect or manual paste
//! 4. Exchange the token for a Windsurf API key via RegisterUser

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use uuid::Uuid;

use super::account::{self, WindsurfAccount};

const WINDSURF_AUTH_BASE_URL: &str = "https://www.windsurf.com";
const WINDSURF_REGISTER_API_BASE_URL: &str = "https://register.windsurf.com";
const WINDSURF_DEFAULT_API_SERVER_URL: &str = "https://server.codeium.com";
const WINDSURF_CLIENT_ID: &str = "3GUryQ7ldAeKEuD2obYnppsnmj58eP5u";
const USER_AGENT: &str = "cc-switch-windsurf-oauth";
const OAUTH_TIMEOUT_SECONDS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval_seconds: u64,
    pub callback_url: Option<String>,
}

#[derive(Clone)]
struct PendingOAuthState {
    login_id: String,
    state: String,
    auth_url: String,
    callback_url: String,
    port: u16,
    expires_at: i64,
    access_token: Option<String>,
    callback_error: Option<String>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Clone)]
struct CallbackServerState {
    expected_login_id: String,
    expected_state: String,
}

static PENDING: LazyLock<Arc<Mutex<Option<PendingOAuthState>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(None)));

fn now_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

fn generate_token() -> String {
    Uuid::new_v4().simple().to_string()
}

fn set_pending(state: Option<PendingOAuthState>) {
    if let Ok(mut guard) = PENDING.lock() {
        if let Some(previous) = guard.take() {
            if let Ok(mut shutdown) = previous.shutdown.lock() {
                if let Some(sender) = shutdown.take() {
                    let _ = sender.send(());
                }
            }
        }
        *guard = state;
    }
}

fn clone_pending() -> Option<PendingOAuthState> {
    PENDING.lock().ok().and_then(|guard| guard.clone())
}

fn clear_pending_if_matches(login_id: &str, state: &str) {
    let should_clear = clone_pending()
        .map(|current| current.login_id == login_id && current.state == state)
        .unwrap_or(false);
    if should_clear {
        set_pending(None);
    }
}

fn update_pending_token(login_id: &str, expected_state: &str, access_token: String) -> bool {
    let Ok(mut guard) = PENDING.lock() else {
        return false;
    };
    let Some(current) = guard.as_mut() else {
        return false;
    };
    if current.login_id != login_id || current.state != expected_state {
        return false;
    }
    current.access_token = Some(access_token);
    current.callback_error = None;
    true
}

fn update_pending_error(login_id: &str, expected_state: &str, message: String) -> bool {
    let Ok(mut guard) = PENDING.lock() else {
        return false;
    };
    let Some(current) = guard.as_mut() else {
        return false;
    };
    if current.login_id != login_id || current.state != expected_state {
        return false;
    }
    current.callback_error = Some(message);
    true
}

fn build_auth_url(redirect_uri: &str, state: &str) -> String {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params.append_pair("response_type", "token");
    params.append_pair("client_id", WINDSURF_CLIENT_ID);
    params.append_pair("redirect_uri", redirect_uri);
    params.append_pair("state", state);
    params.append_pair("prompt", "login");
    params.append_pair("redirect_parameters_type", "query");
    params.append_pair("workflow", "onboarding");
    format!(
        "{}/windsurf/signin?{}",
        WINDSURF_AUTH_BASE_URL,
        params.finish()
    )
}

fn to_start_response(state: &PendingOAuthState) -> WindsurfOAuthStartResponse {
    WindsurfOAuthStartResponse {
        login_id: state.login_id.clone(),
        verification_uri: state.auth_url.clone(),
        verification_uri_complete: Some(state.auth_url.clone()),
        expires_in: (state.expires_at - now_timestamp()).max(0) as u64,
        interval_seconds: 1,
        callback_url: Some(state.callback_url.clone()),
    }
}

fn decode_query_component(value: &str) -> String {
    percent_decode(value).unwrap_or_else(|| value.to_string())
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hi = from_hex(bytes[index + 1])?;
                let lo = from_hex(bytes[index + 2])?;
                out.push((hi << 4) | lo);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?.trim();
            if key.is_empty() {
                return None;
            }
            let value = parts.next().unwrap_or("");
            Some((key.to_string(), decode_query_component(value)))
        })
        .collect()
}

fn parse_callback_url(raw_callback_url: &str, port: u16) -> Result<url::Url, String> {
    let trimmed = raw_callback_url.trim();
    if trimmed.is_empty() {
        return Err("回调链接不能为空".to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return url::Url::parse(trimmed).map_err(|error| format!("回调链接格式无效: {error}"));
    }
    if trimmed.starts_with('/') {
        return url::Url::parse(&format!("http://127.0.0.1:{port}{trimmed}"))
            .map_err(|error| format!("回调链接格式无效: {error}"));
    }
    url::Url::parse(&format!(
        "http://127.0.0.1:{port}/windsurf-auth-callback?{}",
        trimmed.trim_start_matches('?')
    ))
    .map_err(|error| format!("回调链接格式无效: {error}"))
}

fn oauth_success_html() -> Html<&'static str> {
    Html(
        r#"<!DOCTYPE html>
<html><head><meta charset="UTF-8" /><title>Windsurf 授权成功</title>
<style>
body{font-family:-apple-system,BlinkMacSystemFont,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#0f172a;color:#e2e8f0}
.box{text-align:center;max-width:460px;padding:24px;border-radius:12px;background:#111827;border:1px solid #1f2937}
h1{margin:0 0 10px;color:#22c55e;font-size:24px}
</style></head>
<body><div class="box"><h1>授权成功</h1><p>你可以关闭此页面并返回 CC Switch。</p></div></body></html>"#,
    )
}

fn escape_html(message: &str) -> String {
    let mut escaped = String::with_capacity(message.len());
    for character in message.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn oauth_fail_html(message: &str) -> Html<String> {
    Html(format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="UTF-8" /><title>Windsurf 授权失败</title>
<style>
body{{font-family:-apple-system,BlinkMacSystemFont,sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#0f172a;color:#e2e8f0}}
.box{{text-align:center;max-width:520px;padding:24px;border-radius:12px;background:#111827;border:1px solid #1f2937}}
h1{{margin:0 0 10px;color:#ef4444;font-size:24px}}
p{{margin:0;opacity:.92;word-break:break-word}}
</style></head>
<body><div class="box"><h1>授权失败</h1><p>{}</p></div></body></html>"#,
        escape_html(message)
    ))
}

fn pick_string(obj: Option<&Value>, keys: &[&str]) -> Option<String> {
    let object = obj?.as_object()?;
    for key in keys {
        if let Some(value) = object.get(*key) {
            match value {
                Value::String(text) if !text.trim().is_empty() => {
                    return Some(text.trim().to_string());
                }
                Value::Number(number) => return Some(number.to_string()),
                _ => {}
            }
        }
    }
    None
}

async fn handle_callback(
    State(server): State<CallbackServerState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let state = params.get("state").cloned().unwrap_or_default();
    let access_token = params.get("access_token").cloned().unwrap_or_default();
    let error = params.get("error").cloned();
    let error_desc = params.get("error_description").cloned().unwrap_or_default();

    if state != server.expected_state {
        return (
            StatusCode::BAD_REQUEST,
            oauth_fail_html("state 校验失败，请重新授权。"),
        )
            .into_response();
    }

    if let Some(error) = error {
        let message = if error_desc.is_empty() {
            format!("授权失败: {error}")
        } else {
            format!("授权失败: {error} ({error_desc})")
        };
        let _ = update_pending_error(
            &server.expected_login_id,
            &server.expected_state,
            message.clone(),
        );
        return (StatusCode::BAD_REQUEST, oauth_fail_html(&message)).into_response();
    }

    if access_token.trim().is_empty() {
        let message = "回调缺少 access_token，请重新授权。";
        let _ = update_pending_error(
            &server.expected_login_id,
            &server.expected_state,
            message.to_string(),
        );
        return (StatusCode::BAD_REQUEST, oauth_fail_html(message)).into_response();
    }

    if !update_pending_token(
        &server.expected_login_id,
        &server.expected_state,
        access_token,
    ) {
        return (
            StatusCode::GONE,
            oauth_fail_html("登录会话已失效，请重新发起授权。"),
        )
            .into_response();
    }

    (StatusCode::OK, oauth_success_html()).into_response()
}

async fn handle_cancel() -> impl IntoResponse {
    (StatusCode::OK, "cancelled")
}

async fn handle_not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("Not Found"))
        .unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response())
}

async fn start_callback_server(
    listener: TcpListener,
    expected_login_id: String,
    expected_state: String,
    shutdown_rx: oneshot::Receiver<()>,
) {
    let app = Router::new()
        .route("/windsurf-auth-callback", get(handle_callback))
        .route("/cancel", get(handle_cancel))
        .fallback(handle_not_found)
        .with_state(CallbackServerState {
            expected_login_id: expected_login_id.clone(),
            expected_state: expected_state.clone(),
        });

    let port = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or_default();
    log::info!("[Windsurf OAuth] callback server listening on http://127.0.0.1:{port}");
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = shutdown_rx.await;
    });

    if let Err(error) = server.await {
        log::error!(
            "[Windsurf OAuth] callback server stopped with error: login_id={expected_login_id}, error={error}"
        );
    }
}

fn spawn_callback_server(
    listener: TcpListener,
    login_id: String,
    state: String,
) -> oneshot::Sender<()> {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let timeout_login_id = login_id.clone();
    let timeout_state = state.clone();
    tokio::spawn(async move {
        start_callback_server(listener, login_id, state, shutdown_rx).await;
    });
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(OAUTH_TIMEOUT_SECONDS)).await;
        if let Some(pending) = clone_pending() {
            if pending.login_id == timeout_login_id
                && pending.state == timeout_state
                && pending.access_token.is_none()
                && pending.callback_error.is_none()
            {
                log::warn!("[Windsurf OAuth] waiting timed out: login_id={timeout_login_id}");
                let _ = update_pending_error(
                    &timeout_login_id,
                    &timeout_state,
                    "等待 Windsurf 授权超时，请重新发起授权".to_string(),
                );
            }
        }
    });
    shutdown_tx
}

async fn bind_callback_listener() -> Result<(TcpListener, u16), String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("无法绑定本地 OAuth 回调端口: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("读取本地 OAuth 回调端口失败: {error}"))?
        .port();
    Ok((listener, port))
}

fn notify_cancel(port: u16) {
    let url = format!("http://127.0.0.1:{port}/cancel");
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build();
        if let Ok(client) = client {
            let _ = client.get(url).send().await;
        }
    });
}

pub async fn start_login() -> Result<WindsurfOAuthStartResponse, String> {
    if let Some(state) = clone_pending() {
        if state.expires_at > now_timestamp() {
            log::info!(
                "[Windsurf OAuth] reuse pending login: login_id={}, port={}",
                state.login_id,
                state.port
            );
            return Ok(to_start_response(&state));
        }
    }
    set_pending(None);

    let (listener, port) = bind_callback_listener().await?;
    let login_id = generate_token();
    let state_token = generate_token();
    let callback_url = format!("http://127.0.0.1:{port}/windsurf-auth-callback");
    let auth_url = build_auth_url(&callback_url, &state_token);
    let shutdown_tx = spawn_callback_server(listener, login_id.clone(), state_token.clone());

    let pending = PendingOAuthState {
        login_id: login_id.clone(),
        state: state_token,
        auth_url,
        callback_url: callback_url.clone(),
        port,
        expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS as i64,
        access_token: None,
        callback_error: None,
        shutdown: Arc::new(Mutex::new(Some(shutdown_tx))),
    };
    set_pending(Some(pending.clone()));
    log::info!(
        "[Windsurf OAuth] login session created: login_id={}, callback_url={}",
        pending.login_id,
        pending.callback_url
    );
    Ok(to_start_response(&pending))
}

pub async fn complete_login(login_id: &str) -> Result<WindsurfAccount, String> {
    let token = loop {
        let state = clone_pending().ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
        if state.login_id != login_id {
            return Err("登录会话已变更，请刷新后重试".to_string());
        }
        if state.expires_at <= now_timestamp() {
            clear_pending_if_matches(&state.login_id, &state.state);
            return Err("等待 Windsurf 授权超时，请重新发起授权".to_string());
        }
        if let Some(error) = state.callback_error {
            clear_pending_if_matches(&state.login_id, &state.state);
            return Err(error);
        }
        if let Some(token) = state.access_token.clone() {
            break (state, token);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };

    let (state, access_token) = token;
    let account = build_account_from_firebase_token(&access_token).await;
    clear_pending_if_matches(&state.login_id, &state.state);
    account
}

pub fn cancel_login(login_id: Option<&str>) -> Result<(), String> {
    let current = clone_pending();
    match (current.as_ref(), login_id) {
        (Some(state), Some(input)) if state.login_id != input => {
            return Err("登录会话不匹配，取消失败".to_string());
        }
        (Some(state), _) => {
            notify_cancel(state.port);
            set_pending(None);
        }
        (None, _) => {}
    }
    Ok(())
}

pub fn submit_callback_url(login_id: &str, callback_url: &str) -> Result<(), String> {
    let pending = clone_pending().ok_or_else(|| "登录流程已取消，请重新发起授权".to_string())?;
    if pending.login_id != login_id {
        return Err("登录会话已变更，请刷新后重试".to_string());
    }
    if pending.expires_at <= now_timestamp() {
        return Err("等待 Windsurf 授权超时，请重新发起授权".to_string());
    }

    let parsed = parse_callback_url(callback_url, pending.port)?;
    if parsed.path() != "/windsurf-auth-callback" {
        return Err("回调链接路径无效，必须为 /windsurf-auth-callback".to_string());
    }

    let params = parse_query_params(parsed.query().unwrap_or_default());
    let state = params
        .get("state")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "回调链接中缺少 state 参数".to_string())?;
    if state != pending.state {
        return Err("回调 state 校验失败，请确认粘贴的是当前登录会话链接".to_string());
    }

    if let Some(error) = params
        .get("error")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        let error_desc = params
            .get("error_description")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        let message = if error_desc.is_empty() {
            format!("授权失败: {error}")
        } else {
            format!("授权失败: {error} ({error_desc})")
        };
        let _ = update_pending_error(login_id, &pending.state, message.clone());
        return Err(message);
    }

    let access_token = params
        .get("access_token")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "回调链接中缺少 access_token 参数".to_string())?
        .to_string();

    if !update_pending_token(login_id, &pending.state, access_token) {
        return Err("登录流程已取消，请重新发起授权".to_string());
    }
    log::info!("[Windsurf OAuth] accepted manual callback url: login_id={login_id}");
    Ok(())
}

async fn post_seat_management_json(
    base_url: &str,
    method: &str,
    body: Value,
) -> Result<Value, String> {
    let base = base_url.trim().trim_end_matches('/');
    let url = format!("{base}/exa.seat_management_pb.SeatManagementService/{method}");
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("构建 Windsurf OAuth 客户端失败: {error}"))?;

    let response = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("请求 Windsurf {method} 失败: {error}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "请求 Windsurf {method} 失败: status={}, body_len={}",
            status.as_u16(),
            text.len()
        ));
    }
    serde_json::from_str::<Value>(&text).map_err(|error| {
        format!(
            "解析 Windsurf {method} 响应失败: {error} (body_len={})",
            text.len()
        )
    })
}

async fn register_user(
    firebase_id_token: &str,
) -> Result<(String, String, Option<String>), String> {
    let value = post_seat_management_json(
        WINDSURF_REGISTER_API_BASE_URL,
        "RegisterUser",
        json!({ "firebase_id_token": firebase_id_token }),
    )
    .await?;
    let api_key = pick_string(Some(&value), &["apiKey", "api_key"])
        .ok_or_else(|| "RegisterUser 响应缺少 apiKey".to_string())?;
    let api_server_url = pick_string(Some(&value), &["apiServerUrl", "api_server_url"])
        .unwrap_or_else(|| WINDSURF_DEFAULT_API_SERVER_URL.to_string());
    let name = pick_string(Some(&value), &["name"]);
    Ok((api_key, api_server_url, name))
}

async fn get_user_status(api_server_url: &str, api_key: &str) -> Result<Value, String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    post_seat_management_json(
        api_server_url,
        "GetUserStatus",
        json!({
            "metadata": {
                "apiKey": api_key,
                "ideName": "Windsurf",
                "ideVersion": "1.0.0",
                "extensionName": "codeium.windsurf",
                "extensionVersion": "1.0.0",
                "locale": "zh-CN",
                "os": os,
                "disableTelemetry": false,
                "sessionId": format!("cc-switch-{}", now_timestamp()),
                "requestId": now_timestamp().to_string()
            }
        }),
    )
    .await
}

async fn build_account_from_firebase_token(
    firebase_id_token: &str,
) -> Result<WindsurfAccount, String> {
    let (api_key, api_server_url, register_name) = register_user(firebase_id_token).await?;
    let user_status = match get_user_status(&api_server_url, &api_key).await {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("[Windsurf OAuth] GetUserStatus failed (continuing): {error}");
            None
        }
    };

    let email = user_status
        .as_ref()
        .and_then(|value| pick_string(value.get("user"), &["email"]))
        .or_else(|| {
            user_status
                .as_ref()
                .and_then(|value| pick_string(Some(value), &["email"]))
        });
    let name = register_name
        .or_else(|| {
            user_status
                .as_ref()
                .and_then(|value| pick_string(value.get("user"), &["name"]))
        })
        .or_else(|| {
            user_status
                .as_ref()
                .and_then(|value| pick_string(Some(value), &["name"]))
        })
        .or_else(|| email.clone())
        .unwrap_or_else(|| "Windsurf Account".to_string());

    Ok(account::new_account_from_oauth(
        api_key,
        api_server_url,
        Some(name),
        email,
        user_status,
    ))
}
