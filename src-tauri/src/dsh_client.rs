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
    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body
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

    // 读响应：先读 head 拿 Content-Length，再读 body
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    // 简单粗暴：读完整个连接直到 EOF（服务端 Connection: close）
    loop {
        let n = stream.read(&mut chunk).map_err(|e| DshError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf).to_string();

    // 分离 head/body
    let split = text.find("\r\n\r\n").ok_or_else(|| DshError::Http("no header separator".into()))?;
    let head = &text[..split];
    let body_text = &text[split + 4..];

    // 状态码
    let status_line = head.lines().next().unwrap_or("");
    if !status_line.contains(" 200") {
        return Err(DshError::Http(format!("status {status_line}: {body_text}")));
    }

    // 处理 chunked transfer-encoding（server 可能用 a0\r\n...\r\n0\r\n\r\n 分块）
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let body_text = if chunked {
        decode_chunked(body_text).map_err(DshError::Http)?
    } else {
        body_text.to_string()
    };

    let v: Value = serde_json::from_str(&body_text)
        .map_err(|e| DshError::Http(format!("bad json: {e}; raw={body_text}")))?;

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
