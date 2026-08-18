//! DSH Web RPC client — 通过 HTTP 驱动 DSH agent 写小说。
//!
//! DSH web (127.0.0.1:3080) 暴露 JSON-RPC over HTTP：
//!   POST /api/<method>
//!   body: { "type":"client-request", "rpcId":"...", "method":"<method>", "payload":{...} }
//!   resp: { "type":"server-response","rpcId":"...","result":{"ok":true,"value":...} }
//!
//! 关键方法：
//!   session.create   (cwd / workspaceId / sessionId / agentPreset)
//!   session.prompt   (sessionId, mode: queue|steer, content:[{type:text,text}])
//!   session.history  (sessionId, beforeSeq?, maxMessages?) → { events:[{event:{type,seq,data}}] }
//!   session.cancel   (sessionId)
//!
//! 我们不引入 reqwest —— 用 std::net::TcpStream 写最简 HTTP/1.1 JSON POST，
//! 保持零额外依赖。

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const DEFAULT_PORT: u16 = 3080;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug)]
pub enum DshError {
    Http(String),
    Rpc(String),
    Io(String),
}

impl std::fmt::Display for DshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DshError::Http(s) => write!(f, "HTTP: {s}"),
            DshError::Rpc(s) => write!(f, "RPC: {s}"),
            DshError::Io(s) => write!(f, "IO: {s}"),
        }
    }
}

impl std::error::Error for DshError {}

/// 原始 HTTP 响应（head 原文 + 解码后的 body）。
struct HttpResponse {
    status: u16,
    head: String,
    body: String,
}

/// 发一个 POST，返回原始响应（不判状态码）。支持可选 Cookie 头。
/// `path` 形如 `/api/session.create` 或 `/auth/login`。
fn http_post_raw(
    port: u16,
    path: &str,
    body_str: &str,
    cookie: Option<&str>,
    timeout_ms: u64,
) -> Result<HttpResponse, DshError> {
    let cookie_line = match cookie {
        Some(c) if !c.is_empty() => format!("Cookie: {c}\r\n"),
        _ => String::new(),
    };
    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         {}Connection: close\r\n\
         \r\n\
         {}",
        body_str.len(),
        cookie_line,
        body_str
    );

    let addr = format!("127.0.0.1:{port}");
    let mut stream = TcpStream::connect(&addr).map_err(|e| DshError::Io(e.to_string()))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
        .map_err(|e| DshError::Io(e.to_string()))?;
    stream
        .write_all(req.as_bytes())
        .map_err(|e| DshError::Io(e.to_string()))?;
    stream
        .flush()
        .map_err(|e| DshError::Io(e.to_string()))?;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk).map_err(|e| DshError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf).to_string();

    let split = text.find("\r\n\r\n").ok_or_else(|| DshError::Http("no header separator".into()))?;
    let head = text[..split].to_string();
    let body_text = &text[split + 4..];

    let status_line = head.lines().next().unwrap_or("");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let body = if chunked {
        decode_chunked(body_text).map_err(DshError::Http)?
    } else {
        body_text.to_string()
    };

    Ok(HttpResponse { status, head, body })
}

/// 从响应头里提取 Set-Cookie 的第一个 `name=value`（不含属性）。
fn extract_cookie(head: &str) -> Option<String> {
    for line in head.lines() {
        if !line.to_ascii_lowercase().starts_with("set-cookie:") {
            continue;
        }
        let v = line.trim_start_matches(|c| c == ' ' || c == '\t');
        let v = v.strip_prefix("Set-Cookie:").or_else(|| v.strip_prefix("set-cookie:"))?;
        let v = v.trim();
        // 取第一个 ; 之前的 name=value 部分
        let pair = v.split(';').next().unwrap_or(v).trim();
        if pair.contains('=') {
            return Some(pair.to_string());
        }
    }
    None
}

/// 加载已保存的 DSH Web 会话 cookie（登录后持久化到 ~/.dsh/novel-studio-desktop.cookie）。
pub fn load_cookie() -> Option<String> {
    let path = cookie_path();
    let text = std::fs::read_to_string(&path).ok()?;
    let text = text.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn cookie_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home)
        .join(".dsh")
        .join("novel-studio-desktop.cookie")
}

fn save_cookie(cookie: &str) -> std::io::Result<()> {
    let path = cookie_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, cookie.as_bytes())
}

/// 清除已保存的会话 cookie（登出）。
pub fn clear_cookie() {
    let _ = std::fs::remove_file(cookie_path());
}

/// 登录结果：可能直接拿到 cookie，也可能需要再输 TOTP 码。
#[derive(Debug)]
pub struct LoginOutcome {
    pub ok: bool,
    pub mfa_required: bool,
    pub mfa_token: Option<String>,
    pub cookie: Option<String>,
    pub message: String,
}

/// 登录 DSH Web：先账号密码，若要求 TOTP 再用 code 换 cookie。
pub fn login(
    username: &str,
    password: &str,
    code: Option<&str>,
    mfa_token: Option<&str>,
    port: u16,
) -> LoginOutcome {
    // 二步：用 mfaToken + code 换 session
    let path = if code.is_some() || mfa_token.is_some() {
        "/auth/mfa/login"
    } else {
        "/auth/login"
    };
    let payload = if code.is_some() || mfa_token.is_some() {
        json!({ "mfaToken": mfa_token.unwrap_or(""), "code": code.unwrap_or("") })
    } else {
        json!({ "username": username, "password": password })
    };

    let resp = match http_post_raw(port, path, &payload.to_string(), None, 15_000) {
        Ok(r) => r,
        Err(e) => {
            return LoginOutcome {
                ok: false,
                mfa_required: false,
                mfa_token: None,
                cookie: None,
                message: e.to_string(),
            }
        }
    };

    let data: Value = match serde_json::from_str(&resp.body) {
        Ok(v) => v,
        Err(_) => {
            return LoginOutcome {
                ok: false,
                mfa_required: false,
                mfa_token: None,
                cookie: None,
                message: format!("登录响应解析失败（status {}）", resp.status),
            }
        }
    };

    // 需要 TOTP 二步
    if data.get("mfaRequired").and_then(|v| v.as_bool()).unwrap_or(false) {
        let token = data.get("mfaToken").and_then(|v| v.as_str()).map(|s| s.to_string());
        return LoginOutcome {
            ok: false,
            mfa_required: true,
            mfa_token: token,
            cookie: None,
            message: "需要二步验证码（TOTP）".into(),
        };
    }

    // 成功：从 Set-Cookie 提取会话 cookie
    if resp.status == 200 {
        if let Some(cookie) = extract_cookie(&resp.head) {
            let _ = save_cookie(&cookie);
            return LoginOutcome {
                ok: true,
                mfa_required: false,
                mfa_token: None,
                cookie: Some(cookie),
                message: "已登录".into(),
            };
        }
    }

    let err = data.get("error").and_then(|v| v.as_str()).unwrap_or("登录失败");
    LoginOutcome {
        ok: false,
        mfa_required: false,
        mfa_token: None,
        cookie: None,
        message: err.to_string(),
    }
}

fn http_post_json(
    port: u16,
    method: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, DshError> {
    let body = json!({
        "type": "client-request",
        "rpcId": format!("rpc-{}", std::process::id()),
        "method": method,
        "payload": payload,
    })
    .to_string();

    let path = format!("/api/{method}");
    let cookie = load_cookie();
    let resp = http_post_raw(port, &path, &body, cookie.as_deref(), timeout_ms)?;

    if resp.status != 200 {
        return Err(DshError::Http(format!(
            "status HTTP/1.1 {}: {}",
            resp.status, resp.body
        )));
    }

    let v: Value = serde_json::from_str(&resp.body)
        .map_err(|e| DshError::Http(format!("bad json: {e}; raw={}", resp.body)))?;

    // 检查 envelope
    if v.get("type").and_then(|t| t.as_str()) != Some("server-response") {
        return Err(DshError::Rpc(format!("unexpected envelope: {v}")));
    }
    let result = v.get("result").ok_or_else(|| DshError::Rpc("missing result".into()))?;
    let ok = result.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
    if !ok {
        let err = result.get("error").cloned().unwrap_or(Value::Null);
        return Err(DshError::Rpc(format!("{err}")));
    }
    let value = result.get("value").cloned().unwrap_or(Value::Null);
    Ok(value)
}

/// 创建或复用会话。若传 session_id 且已存在则直接返回（不重复创建）。
pub fn session_create(
    cwd: &str,
    session_id: Option<&str>,
    agent_preset: Option<&str>,
    port: u16,
) -> Result<String, DshError> {
    let mut payload = serde_json::Map::new();
    payload.insert("cwd".into(), Value::String(cwd.to_string()));
    if let Some(sid) = session_id {
        payload.insert("sessionId".into(), Value::String(sid.to_string()));
    }
    if let Some(p) = agent_preset {
        payload.insert("agentPreset".into(), Value::String(p.to_string()));
    }
    let v = http_post_json(port, "session.create", Value::Object(payload), DEFAULT_TIMEOUT_MS)?;
    let sid = v
        .get("sessionId")
        .and_then(|s| s.as_str())
        .ok_or_else(|| DshError::Rpc("session.create no sessionId".into()))?
        .to_string();
    Ok(sid)
}

/// 往会话里提交一个 prompt（queue 模式，异步）。
pub fn session_prompt(
    session_id: &str,
    text: &str,
    mode: &str,
    port: u16,
) -> Result<(), DshError> {
    let payload = json!({
        "sessionId": session_id,
        "mode": mode,
        "content": [ { "type": "text", "text": text } ],
    });
    let v = http_post_json(port, "session.prompt", payload, DEFAULT_TIMEOUT_MS)?;
    let accepted = v.get("accepted").and_then(|a| a.as_bool()).unwrap_or(false);
    if !accepted {
        return Err(DshError::Rpc("session.prompt not accepted".into()));
    }
    Ok(())
}

/// 拉取会话历史事件。返回全部 events 数组（按 seq 升序）。
pub fn session_history(session_id: &str, port: u16) -> Result<Vec<Value>, DshError> {
    let payload = json!({ "sessionId": session_id });
    let v = http_post_json(port, "session.history", payload, DEFAULT_TIMEOUT_MS)?;
    let events = v
        .get("events")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(events)
}

/// 轮询直到 assistant 消息出现（或超时）。
/// 返回最后的 assistant 文本 + 是否有抉择点标记。
pub struct WriteOutcome {
    pub text: String,
    pub choice_request: Option<Value>,
    pub turn_ended: bool,
}

pub fn wait_for_assistant(
    session_id: &str,
    port: u16,
    timeout_secs: u64,
) -> Result<WriteOutcome, DshError> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_text = String::new();
    let mut saw_turn_end = false;

    loop {
        if Instant::now() > deadline {
            return Ok(WriteOutcome {
                text: last_text,
                choice_request: None,
                turn_ended: saw_turn_end,
            });
        }

        let events = match session_history(session_id, port) {
            Ok(e) => e,
            Err(e) => {
                // web 服务可能还没起来 / 暂时不可达 —— 重试
                if Instant::now() > deadline {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        };

        for ev in &events {
            let ev = ev.get("event").cloned().unwrap_or(Value::Null);
            let t = ev.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match t {
                "assistant/message" => {
                    if let Some(msg) = ev.pointer("/data/message") {
                        if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                            let mut text = String::new();
                            for c in content {
                                if c.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    if let Some(txt) = c.get("text").and_then(|t| t.as_str()) {
                                        text.push_str(txt);
                                    }
                                }
                            }
                            if !text.is_empty() {
                                last_text = text;
                            }
                        }
                    }
                }
                "turn/end" => {
                    saw_turn_end = true;
                }
                _ => {}
            }
        }

        if saw_turn_end {
            // 看看 text 里是否有抉择点 JSON 标记
            let choice = extract_choice_request(&last_text);
            return Ok(WriteOutcome {
                text: last_text,
                choice_request: choice,
                turn_ended: true,
            });
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

/// 从 agent 输出的文本里解析抉择点 JSON（如果它输出 `@@CHOICE@@ {json} @@END@@`）。
fn extract_choice_request(text: &str) -> Option<Value> {
    let start_marker = "@@CHOICE@@";
    let end_marker = "@@END@@";
    let start = text.find(start_marker)?;
    let rest = &text[start + start_marker.len()..];
    let end = rest.find(end_marker)?;
    let json_str = rest[..end].trim();
    serde_json::from_str(json_str).ok()
}

/// 检查 web 服务是否活着。
pub fn ping(port: u16) -> bool {
    match TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), Duration::from_millis(1000)) {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// 解码 HTTP chunked body。
/// 输入形如：`a0\r\n{data}\r\n0\r\n\r\n`
fn decode_chunked(text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    loop {
        // 找 chunk size 行结束（\r\n）
        let line_end = rest.find("\r\n").ok_or_else(|| "chunk: missing size line".to_string())?;
        let size_str = rest[..line_end].trim();
        // 可能带分号扩展：`a0;ext=1`
        let size_hex = size_str.split(';').next().unwrap_or(size_str).trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|e| format!("chunk size parse '{size_hex}': {e}"))?;
        if size == 0 {
            break; // 终止 chunk
        }
        let data_start = line_end + 2;
        if data_start + size > rest.len() {
            return Err("chunk: data truncated".to_string());
        }
        out.push_str(&rest[data_start..data_start + size]);
        rest = &rest[data_start + size..];
        // 跳过后面的 \r\n
        if rest.starts_with("\r\n") {
            rest = &rest[2..];
        } else {
            // 容错：有些实现没有
        }
    }
    Ok(out)
}

pub fn default_port() -> u16 {
    DEFAULT_PORT
}
