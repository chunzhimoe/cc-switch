//! Devin Auth1 protocol helpers used by Windsurf account login and preflight refresh.
//!
//! The endpoints and protobuf field mapping follow cockpit-tools' Windsurf
//! implementation. Tokens are kept in return values only and are never logged.

use base64::Engine;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

const PASSWORD_LOGIN_URL: &str = "https://windsurf.com/_devin-auth/password/login";
const POST_AUTH_URL: &str =
    "https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/WindsurfPostAuth";
const GET_OTT_URL: &str =
    "https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/GetOneTimeAuthToken";
const REGISTER_USER_URL: &str =
    "https://register.windsurf.com/exa.seat_management_pb.SeatManagementService/RegisterUser";
const GET_CURRENT_USER_URL: &str =
    "https://windsurf.com/_backend/exa.seat_management_pb.SeatManagementService/GetCurrentUser";
pub const USER_STATUS_URL: &str =
    "https://server.self-serve.windsurf.com/exa.seat_management_pb.SeatManagementService/GetUserStatus";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";

#[derive(Debug, Clone)]
pub struct PasswordLoginResult {
    pub auth1_token: String,
    pub user_id: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FullRefreshResult {
    pub ide_token: String,
    pub session_token: String,
    pub auth1_token: String,
    pub account_id: String,
    pub org_id: String,
    pub user_status_proto_b64: Option<String>,
    pub user_status: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct PasswordLoginResponse {
    token: Option<String>,
    user_id: Option<String>,
    email: Option<String>,
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("构建 Windsurf 登录客户端失败: {error}"))
}

pub async fn login_with_password(email: &str, password: &str) -> Result<PasswordLoginResult, String> {
    let email = email.trim();
    if email.is_empty() || password.is_empty() {
        return Err("邮箱和密码不能为空".to_string());
    }
    let response = client()?
        .post(PASSWORD_LOGIN_URL)
        .header("Accept", "*/*")
        .header("Content-Type", "application/json")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/account/login")
        .json(&json!({
            "email": email,
            "password": password,
            "product": "Windsurf",
        }))
        .send()
        .await
        .map_err(|error| format!("Devin 邮密登录请求失败: {error}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => "邮箱或密码错误".to_string(),
            404 => "邮箱未注册".to_string(),
            429 => "请求过于频繁，请稍后再试".to_string(),
            code => format!("Devin 登录失败 (HTTP {code})"),
        });
    }
    let parsed: PasswordLoginResponse = serde_json::from_str(&body)
        .map_err(|error| format!("解析 Devin 登录响应失败: {error}"))?;
    let auth1_token = parsed
        .token
        .filter(|token| token.starts_with("auth1_"))
        .ok_or_else(|| "Devin 登录响应未包含有效 auth1_token".to_string())?;
    Ok(PasswordLoginResult {
        auth1_token,
        user_id: parsed.user_id,
        email: parsed.email,
    })
}

pub async fn full_refresh_from_auth1(auth1_token: &str) -> Result<FullRefreshResult, String> {
    let auth1_token = auth1_token.trim();
    if !auth1_token.starts_with("auth1_") {
        return Err("auth1 凭据格式错误，应以 auth1_ 开头".to_string());
    }
    let http = client()?;
    let (session_token, auth1, account_id, org_id) = post_auth(&http, auth1_token).await?;
    let ott = get_ott(&http, &session_token, &auth1, &account_id, &org_id).await?;
    let ide_token = register_user(&http, &ott).await?;
    let (user_status_proto_b64, user_status) = match get_current_user(
        &http,
        &ide_token,
        &auth1,
        &account_id,
        &org_id,
    )
    .await
    {
        Ok(raw) => (
            Some(base64::engine::general_purpose::STANDARD.encode(raw)),
            None,
        ),
        Err(error) => {
            log::warn!("Windsurf GetCurrentUser failed; continuing without proto: {error}");
            (None, None)
        }
    };
    Ok(FullRefreshResult {
        ide_token,
        session_token,
        auth1_token: auth1,
        account_id,
        org_id,
        user_status_proto_b64,
        user_status,
    })
}

pub async fn fetch_user_status(ide_token: &str) -> Result<Value, String> {
    let token = ide_token.trim();
    if token.is_empty() {
        return Err("ide_token 不能为空".to_string());
    }
    let response = client()?
        .post(USER_STATUS_URL)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "metadata": {
                "ide_name": "WINDSURF",
                "ide_version": "1.0.0",
                "extension_version": "1.0.0",
                "api_key": token,
            }
        }))
        .send()
        .await
        .map_err(|error| format!("GetUserStatus 请求失败: {error}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("GetUserStatus HTTP {}", status.as_u16()));
    }
    serde_json::from_str(&body).map_err(|error| format!("解析 GetUserStatus 响应失败: {error}"))
}

async fn post_auth(
    http: &Client,
    auth1: &str,
) -> Result<(String, String, String, String), String> {
    let body = encode_string_field(1, auth1);
    let mut last_error = String::new();
    for attempt in 1..=3u64 {
        let response = http
            .post(POST_AUTH_URL)
            .header("Content-Type", "application/proto")
            .header("connect-protocol-version", "1")
            .header("Accept", "application/proto")
            .header("Origin", "https://windsurf.com")
            .header("Referer", "https://windsurf.com/account/login")
            .header("x-devin-auth1-token", auth1)
            .body(body.clone())
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                let raw = response
                    .bytes()
                    .await
                    .map_err(|error| format!("读取 PostAuth 响应失败: {error}"))?;
                let session = proto_string(&raw, 1)
                    .filter(|value| value.starts_with("devin-session-token$"))
                    .ok_or_else(|| "PostAuth 响应未含 session token".to_string())?;
                let auth1_back = proto_string(&raw, 3).unwrap_or_else(|| auth1.to_string());
                let account = proto_string(&raw, 4)
                    .ok_or_else(|| "PostAuth 响应未含 account id".to_string())?;
                let org = proto_string(&raw, 5)
                    .ok_or_else(|| "PostAuth 响应未含 organization id".to_string())?;
                return Ok((session, auth1_back, account, org));
            }
            Ok(response) => {
                last_error = format!("HTTP {}", response.status().as_u16());
            }
            Err(error) => last_error = error.to_string(),
        }
        if attempt < 3 {
            tokio::time::sleep(Duration::from_millis(1500 * attempt)).await;
        }
    }
    Err(format!("WindsurfPostAuth 失败: {last_error}"))
}

async fn get_ott(
    http: &Client,
    session: &str,
    auth1: &str,
    account: &str,
    org: &str,
) -> Result<String, String> {
    let body = encode_string_field(1, session);
    for attempt in 1..=3u64 {
        let response = http
            .post(GET_OTT_URL)
            .header("Content-Type", "application/proto")
            .header("connect-protocol-version", "1")
            .header("Accept", "application/proto")
            .header("Origin", "https://windsurf.com")
            .header("Referer", "https://windsurf.com/editor/auth-success")
            .header("x-auth-token", session)
            .header("x-devin-session-token", session)
            .header("x-devin-account-id", account)
            .header("x-devin-primary-org-id", org)
            .header("x-devin-auth1-token", auth1)
            .body(body.clone())
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                let raw = response
                    .bytes()
                    .await
                    .map_err(|error| format!("读取 GetOTT 响应失败: {error}"))?;
                return proto_string(&raw, 1)
                    .filter(|value| value.starts_with("ott$"))
                    .ok_or_else(|| "GetOTT 响应未含 ott".to_string());
            }
            Ok(_) | Err(_) if attempt < 3 => {
                tokio::time::sleep(Duration::from_millis(1500 * attempt)).await;
            }
            Ok(response) => return Err(format!("GetOTT HTTP {}", response.status().as_u16())),
            Err(error) => return Err(format!("GetOTT 请求失败: {error}")),
        }
    }
    Err("GetOTT 重试失败".to_string())
}

async fn register_user(http: &Client, ott: &str) -> Result<String, String> {
    let body = encode_string_field(1, ott);
    for attempt in 1..=3u64 {
        let response = http
            .post(REGISTER_USER_URL)
            .header("Content-Type", "application/proto")
            .header("connect-protocol-version", "1")
            .header("User-Agent", "connect-go/1.18.1 (go1.26.1)")
            .body(body.clone())
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                let raw = response
                    .bytes()
                    .await
                    .map_err(|error| format!("读取 RegisterUser 响应失败: {error}"))?;
                let needle = b"devin-session-token$";
                let Some(start) = raw.windows(needle.len()).position(|part| part == needle) else {
                    return Err("RegisterUser 响应未含 ide token".to_string());
                };
                let mut end = start;
                while end < raw.len() && (0x20..0x7f).contains(&raw[end]) {
                    end += 1;
                }
                return String::from_utf8(raw[start..end].to_vec())
                    .map_err(|error| format!("ide token 编码错误: {error}"));
            }
            Ok(_) | Err(_) if attempt < 3 => {
                tokio::time::sleep(Duration::from_millis(1500 * attempt)).await;
            }
            Ok(response) => return Err(format!("RegisterUser HTTP {}", response.status().as_u16())),
            Err(error) => return Err(format!("RegisterUser 请求失败: {error}")),
        }
    }
    Err("RegisterUser 重试失败".to_string())
}

async fn get_current_user(
    http: &Client,
    ide_token: &str,
    auth1: &str,
    account: &str,
    org: &str,
) -> Result<Vec<u8>, String> {
    let mut body = encode_string_field(1, ide_token);
    body.extend_from_slice(&[0x10, 0x01, 0x20, 0x01]);
    let response = http
        .post(GET_CURRENT_USER_URL)
        .header("Content-Type", "application/proto")
        .header("connect-protocol-version", "1")
        .header("Accept", "application/proto")
        .header("Origin", "https://windsurf.com")
        .header("Referer", "https://windsurf.com/")
        .header("x-auth-token", ide_token)
        .header("x-devin-session-token", ide_token)
        .header("x-devin-account-id", account)
        .header("x-devin-primary-org-id", org)
        .header("x-devin-auth1-token", auth1)
        .body(body)
        .send()
        .await
        .map_err(|error| format!("GetCurrentUser 请求失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("GetCurrentUser HTTP {}", response.status().as_u16()));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("读取 GetCurrentUser 响应失败: {error}"))
}

fn encode_string_field(field: u32, value: &str) -> Vec<u8> {
    let mut result = Vec::new();
    encode_varint(u64::from(field) << 3 | 2, &mut result);
    encode_varint(value.len() as u64, &mut result);
    result.extend_from_slice(value.as_bytes());
    result
}

fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn proto_string(data: &[u8], wanted_field: u32) -> Option<String> {
    let mut offset = 0usize;
    while offset < data.len() {
        let (tag, next) = read_varint(data, offset)?;
        offset = next;
        let wire_type = (tag & 7) as u8;
        let field = (tag >> 3) as u32;
        if wire_type == 2 {
            let (length, content_start) = read_varint(data, offset)?;
            let length = usize::try_from(length).ok()?;
            let content_end = content_start.checked_add(length)?;
            if content_end > data.len() {
                return None;
            }
            if field == wanted_field {
                return String::from_utf8(data[content_start..content_end].to_vec()).ok();
            }
            offset = content_end;
        } else {
            offset = skip_wire_value(data, offset, wire_type)?;
        }
    }
    None
}

fn read_varint(data: &[u8], mut offset: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    while offset < data.len() && shift < 64 {
        let byte = data[offset];
        offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, offset));
        }
        shift += 7;
    }
    None
}

fn skip_wire_value(data: &[u8], offset: usize, wire_type: u8) -> Option<usize> {
    match wire_type {
        0 => read_varint(data, offset).map(|(_, next)| next),
        1 => offset.checked_add(8).filter(|next| *next <= data.len()),
        2 => {
            let (length, content_start) = read_varint(data, offset)?;
            let length = usize::try_from(length).ok()?;
            content_start
                .checked_add(length)
                .filter(|next| *next <= data.len())
        }
        5 => offset.checked_add(4).filter(|next| *next <= data.len()),
        _ => None,
    }
}
