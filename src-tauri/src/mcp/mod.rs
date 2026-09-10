//! Application config adapters. No MCP process is ever launched.
use crate::{
    agents::AgentTarget,
    extensions::{err, files, model::*, ExtResult},
};
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, path::Path};
pub const REDACTED: &str = "[REDACTED]";
const CORE: &[&str] = &["command", "args", "env", "cwd", "url", "headers"];

pub fn read_config(target: &AgentTarget) -> ExtResult<(String, Value, Vec<u8>)> {
    let path = Path::new(&target.mcp_file);
    if !files::exists(path) {
        return Ok(("missing".into(), json!({}), vec![]));
    }
    let bytes = files::read(path)?;
    let value = parse_document(&target.id, &bytes)?;
    Ok((files::hash(&bytes), value, bytes))
}
pub fn parse_document(target: &str, bytes: &[u8]) -> ExtResult<Value> {
    let value: Value = if target == "codex" {
        let text = std::str::from_utf8(bytes).map_err(|_| err("corrupt", "配置不是 UTF-8"))?;
        let doc: toml::Value =
            toml::from_str(text).map_err(|_| err("corrupt", "Codex TOML 损坏；原文件已保留"))?;
        serde_json::to_value(doc).map_err(|_| err("corrupt", "无法读取 Codex 配置"))?
    } else {
        serde_json::from_slice(bytes).map_err(|_| {
            err(
                "corrupt",
                "应用配置不是有效 JSON（暂不支持 JSONC）；原文件已保留",
            )
        })?
    };
    if !value.is_object() || value.get(container(target)).is_some_and(|v| !v.is_object()) {
        return Err(err("corrupt", "MCP 配置容器必须是对象"));
    }
    Ok(value)
}
pub fn container(target: &str) -> &'static str {
    if target == "codex" {
        "mcp_servers"
    } else {
        "mcpServers"
    }
}
pub fn entries<'a>(target: &str, doc: &'a Value) -> Option<&'a Map<String, Value>> {
    doc.get(container(target)).and_then(Value::as_object)
}
pub fn entry_hash(value: &Value) -> String {
    files::hash(&serde_json::to_vec(value).expect("JSON serializes"))
}
pub fn enabled(target: &str, value: &Value) -> bool {
    if target == "codex" {
        value
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    } else {
        true
    }
}
pub fn effective_enabled(target: &str, key: &str, entry: &Value, document: &Value) -> bool {
    if !enabled(target, entry) {
        return false;
    }
    if target == "gemini" {
        if document
            .pointer("/mcp/excluded")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(key)))
        {
            return false;
        }
        if document
            .pointer("/mcp/allowed")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.iter().any(|v| v.as_str() == Some(key)))
        {
            return false;
        }
    }
    true
}
pub fn decode(target: &str, value: &Value) -> ExtResult<(McpConfig, Value)> {
    let mut fields = value
        .as_object()
        .cloned()
        .ok_or_else(|| err("invalid_request", "服务器配置必须为对象"))?;
    let transport = if fields.contains_key("command") {
        McpTransport::Stdio
    } else if target == "gemini" && fields.contains_key("httpUrl") {
        McpTransport::Http
    } else if fields.contains_key("url") {
        if target == "gemini" || fields.get("type").and_then(Value::as_str) == Some("sse") {
            McpTransport::Sse
        } else {
            McpTransport::Http
        }
    } else {
        return Err(err("unsupported", "配置缺少支持的 command / URL"));
    };
    if let Some(type_value) = fields.get("type").and_then(Value::as_str) {
        if !["stdio", "http", "sse"].contains(&type_value) {
            return Err(err("unsupported", "不支持此 transport"));
        }
    }
    fields.remove("type");
    if target == "codex" {
        fields.remove("enabled");
    }
    if target == "gemini" {
        if let Some(url) = fields.remove("httpUrl") {
            fields.insert("url".into(), url);
        }
    }
    if target == "codex" {
        if let Some(headers) = fields.remove("http_headers") {
            fields.insert("headers".into(), headers);
        }
    }
    let mut core = Map::new();
    for key in CORE {
        if let Some(v) = fields.remove(*key) {
            core.insert((*key).into(), v);
        }
    }
    let config = McpConfig {
        transport,
        fields: Value::Object(core),
    };
    validate(&config)?;
    Ok((config, Value::Object(fields)))
}
pub fn validate(config: &McpConfig) -> ExtResult<()> {
    let fields = config
        .fields
        .as_object()
        .ok_or_else(|| err("invalid_request", "配置必须为 JSON 对象"))?;
    if fields.keys().any(|k| !CORE.contains(&k.as_str())) {
        return Err(err("unsupported", "应用特有字段请放入对应应用的高级字段"));
    }
    if config.transport == McpTransport::Stdio {
        let command = fields
            .get("command")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty() && !s.chars().any(char::is_control))
            .ok_or_else(|| err("invalid_request", "stdio 需要可执行文件名或绝对路径"))?;
        if !Path::new(command).is_absolute()
            && !command
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(err(
                "invalid_request",
                "command 仅填可执行文件名；参数请放在 args",
            ));
        }
        if Path::new(command).is_absolute()
            && Path::new(command).exists()
            && !Path::new(command).is_file()
        {
            return Err(err("invalid_request", "command 指向的不是文件"));
        }
        if fields.contains_key("url") || fields.contains_key("headers") {
            return Err(err("invalid_request", "stdio 不支持远程连接字段"));
        }
    } else {
        let url = fields
            .get("url")
            .and_then(Value::as_str)
            .and_then(|s| reqwest::Url::parse(s).ok())
            .ok_or_else(|| err("invalid_request", "HTTP/SSE 需要有效 URL"))?;
        if !["https", "http"].contains(&url.scheme()) || url.host_str().is_none() {
            return Err(err("invalid_request", "URL 仅支持 HTTP 或 HTTPS"));
        }
        if fields
            .keys()
            .any(|k| ["command", "args", "env", "cwd"].contains(&k.as_str()))
        {
            return Err(err("invalid_request", "远程 transport 不支持进程字段"));
        }
    }
    if fields.get("args").is_some_and(|v| {
        v.as_array()
            .is_none_or(|a| a.iter().any(|v| !v.is_string()))
    }) {
        return Err(err("invalid_request", "args 必须为字符串数组"));
    }
    for key in ["env", "headers"] {
        if fields.get(key).is_some_and(|v| {
            v.as_object()
                .is_none_or(|o| o.values().any(|v| !v.is_string()))
        }) {
            return Err(err("invalid_request", "env/headers 必须为字符串映射"));
        }
    }
    if fields.get("cwd").is_some_and(|v| !v.is_string()) {
        return Err(err("invalid_request", "cwd 必须为路径字符串"));
    }
    if contains_redacted(&config.fields) {
        return Err(err(
            "invalid_request",
            "脱敏占位符不能作为配置值保存；请保留或设置密钥",
        ));
    }
    Ok(())
}
pub fn encode(target: &AgentTarget, config: &McpConfig, extra: &Value) -> ExtResult<Value> {
    validate(config)?;
    if !target.transports.contains(&config.transport) {
        return Err(err("unsupported", "目标应用不支持此 transport"));
    }
    let mut out = extra
        .as_object()
        .cloned()
        .ok_or_else(|| err("invalid_request", "应用高级字段必须为对象"))?;
    if out.keys().any(|k| {
        CORE.contains(&k.as_str())
            || (target.id == "codex" && k == "enabled")
            || ["type", "httpUrl", "http_headers"].contains(&k.as_str())
    }) {
        return Err(err("invalid_request", "高级字段不能覆盖核心配置与启用状态"));
    }
    out.extend(config.fields.as_object().unwrap().clone());
    match target.id.as_str() {
        "codex" => {
            if let Some(h) = out.remove("headers") {
                out.insert("http_headers".into(), h);
            }
        }
        "gemini" if config.transport == McpTransport::Http => {
            if let Some(url) = out.remove("url") {
                out.insert("httpUrl".into(), url);
            }
        }
        "claude" => {
            if config.transport == McpTransport::Stdio && out.contains_key("cwd") {
                return Err(err(
                    "unsupported",
                    "Claude Code 用户级 MCP 不支持 cwd 无损映射",
                ));
            }
            out.insert("type".into(), json!(config.transport));
        }
        _ => {}
    }
    Ok(Value::Object(out))
}
pub fn render(
    target: &str,
    original: &[u8],
    changes: &BTreeMap<String, Option<Value>>,
) -> ExtResult<Vec<u8>> {
    if target == "codex" {
        let text = std::str::from_utf8(original).map_err(|_| err("corrupt", "TOML 编码无效"))?;
        let mut doc: toml_edit::DocumentMut =
            text.parse().map_err(|_| err("corrupt", "TOML 损坏"))?;
        if !doc.contains_key("mcp_servers") {
            doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = doc["mcp_servers"]
            .as_table_like_mut()
            .ok_or_else(|| err("corrupt", "mcp_servers 不是表"))?;
        for (key, value) in changes {
            match value {
                None => {
                    table.remove(key);
                }
                Some(value) => {
                    let new = toml_edit::ser::to_document(value)
                        .map_err(|_| err("unsupported", "高级字段不能转换为 TOML"))?
                        .into_table();
                    if let Some(existing) =
                        table.get_mut(key).and_then(toml_edit::Item::as_table_mut)
                    {
                        merge_table(existing, new);
                    } else {
                        table.insert(key, toml_edit::Item::Table(new));
                    }
                }
            }
        }
        Ok(doc.to_string().into_bytes())
    } else {
        let mut doc = if original.is_empty() {
            json!({})
        } else {
            parse_document(target, original)?
        };
        if doc.get("mcpServers").is_none() {
            doc["mcpServers"] = json!({});
        }
        let map = doc["mcpServers"]
            .as_object_mut()
            .ok_or_else(|| err("corrupt", "mcpServers 不是对象"))?;
        for (key, value) in changes {
            match value {
                Some(v) => {
                    map.insert(key.clone(), v.clone());
                }
                None => {
                    map.remove(key);
                }
            }
        }
        serde_json::to_vec_pretty(&doc).map_err(|_| err("failed", "JSON 序列化失败"))
    }
}
fn merge_table(existing: &mut toml_edit::Table, new: toml_edit::Table) {
    let removed: Vec<_> = existing
        .iter()
        .filter(|(key, _)| !new.contains_key(key))
        .map(|(key, _)| key.to_owned())
        .collect();
    for key in removed {
        existing.remove(&key);
    }
    for (key, mut item) in new {
        if let (Some(old), Some(next)) = (
            existing
                .get_mut(&key)
                .and_then(toml_edit::Item::as_table_mut),
            item.as_table()
                .cloned()
                .or_else(|| item.as_inline_table().map(|t| t.clone().into_table())),
        ) {
            merge_table(old, next);
            continue;
        }
        if let (Some(old), Some(next)) = (
            existing.get(&key).and_then(toml_edit::Item::as_value),
            item.as_value_mut(),
        ) {
            if old.as_str().is_some() && old.as_str() == next.as_str() {
                continue;
            }
            *next.decor_mut() = old.decor().clone();
        }
        existing.insert(&key, item);
    }
}
pub fn redact(value: &Value) -> Value {
    redact_inner(value, true)
}
fn redact_inner(value: &Value, root: bool) -> Value {
    match value {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if root && k == "command" && v.is_string() {
                            v.clone()
                        } else {
                            redact_inner(v, false)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|v| redact_inner(v, false)).collect()),
        Value::String(_) => json!(REDACTED),
        _ => value.clone(),
    }
}
pub fn redacted_json(value: &Value) -> String {
    serde_json::to_string_pretty(&redact(value)).unwrap_or_default()
}
pub fn contains_redacted(v: &Value) -> bool {
    match v {
        Value::String(s) => s == REDACTED,
        Value::Array(a) => a.iter().any(contains_redacted),
        Value::Object(o) => o.values().any(contains_redacted),
        _ => false,
    }
}
fn preserve(new: &mut Value, old: Option<&Value>) -> ExtResult<()> {
    match new {
        Value::String(s) if s == REDACTED => {
            *new = old
                .cloned()
                .ok_or_else(|| err("invalid_request", "新字段不能保留不存在的密钥"))?;
        }
        Value::Object(map) => {
            for (k, v) in map {
                preserve(v, old.and_then(|o| o.get(k)))?;
            }
        }
        Value::Array(list) => {
            for (i, v) in list.iter_mut().enumerate() {
                preserve(v, old.and_then(|o| o.get(i)))?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn parse_edit(edit: &McpEdit, old: Option<&McpRecord>) -> ExtResult<McpConfig> {
    files::safe_name(&edit.name)?;
    let mut fields: Value = serde_json::from_str(&edit.config_json)
        .map_err(|_| err("invalid_request", "配置 JSON 无效"))?;
    preserve(&mut fields, old.map(|r| &r.config.fields))?;
    for (pointer, action) in &edit.secrets {
        if !pointer.starts_with('/') || pointer == "/command" {
            return Err(err("invalid_request", "密钥字段需要 JSON pointer"));
        }
        match action {
            SecretEdit::Keep => {
                let value = old
                    .and_then(|r| r.config.fields.pointer(pointer))
                    .cloned()
                    .ok_or_else(|| err("invalid_request", "保留的密钥不存在"))?;
                *fields
                    .pointer_mut(pointer)
                    .ok_or_else(|| err("invalid_request", "密钥字段不存在"))? = value;
            }
            SecretEdit::Set(value) => {
                *fields
                    .pointer_mut(pointer)
                    .ok_or_else(|| err("invalid_request", "请先在 JSON 中添加密钥字段"))? =
                    json!(value);
            }
            SecretEdit::Remove => {
                let (parent, key) = pointer.rsplit_once('/').unwrap();
                let key = key.replace("~1", "/").replace("~0", "~");
                fields
                    .pointer_mut(parent)
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| err("invalid_request", "移除密钥仅支持对象字段"))?
                    .remove(&key);
            }
        }
    }
    let config = McpConfig {
        transport: edit.transport.clone(),
        fields,
    };
    validate(&config)?;
    Ok(config)
}
pub fn parse_extra(json: &str, old: Option<&Value>) -> ExtResult<Value> {
    let mut value: Value =
        serde_json::from_str(json).map_err(|_| err("invalid_request", "高级字段 JSON 无效"))?;
    preserve(&mut value, old)?;
    if contains_redacted(&value) {
        return Err(err("invalid_request", "不能保存脱敏占位符"));
    }
    Ok(value)
}
