//! Read-only, independent MCP handshake. This module never touches McpManager,
//! its credential vault, a native CLI session, or tools/call.
use crate::{protocol::{self, JsonRpcNotification}, transport::Session, McpError, Result};
use reqwest::{header::{HeaderMap, HeaderName, HeaderValue}, Client};
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

#[path = "probe_container.rs"]
mod container;
pub use container::{container_plan, ContainerCleanupPlan, ContainerCleanupSpec};
/// Host-only credentials: intentionally not Debug or Serialize.
pub enum ProbeEndpoint {
    Stdio { executable: String, args: Vec<String>, cwd: String, environment: Vec<(String, String)>, container: Option<ContainerCleanupSpec> },
    Http { url: String, headers: Vec<(String, String)>, legacy_sse: bool },
}
pub struct ProbeConnection { session: Session, legacy_sse: bool, container: Option<container::OwnedContainerCleanup>, secrets: Vec<zeroize::Zeroizing<String>> }
#[derive(Clone, Serialize)]
pub struct ProbeTools { pub protocol_version: String, pub tools: Vec<Value> }
impl ProbeConnection {
    /// No await between spawn and transfer of process ownership to the caller.
    pub fn connect(endpoint: ProbeEndpoint) -> Result<Self> {
        match endpoint {
            ProbeEndpoint::Stdio { executable, args, cwd, environment, container } => {
                let secrets = environment.iter().filter(|(_,value)| !value.is_empty()).map(|(_,value)| zeroize::Zeroizing::new(value.clone())).collect();
                let session = Session::stdio_with_env(&executable, &args, Some(&cwd), &environment)?;
                Ok(Self { session, legacy_sse: false, container: container.map(|spec| container::OwnedContainerCleanup::new(spec, environment, cwd)), secrets })
            }
            ProbeEndpoint::Http { url, headers, legacy_sse } => {
                let mut values = HeaderMap::new();
                let mut secrets = Vec::new();
                for (name, value) in headers {
                    if !value.is_empty() { secrets.push(zeroize::Zeroizing::new(value.clone())); if let Some(token) = value.strip_prefix("Bearer ") { secrets.push(zeroize::Zeroizing::new(token.to_owned())); } }
                    let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| McpError::CredentialUnavailable)?;
                    if ["host", "content-length", "connection", "mcp-session-id", "mcp-protocol-version", "accept", "content-type"].contains(&name.as_str()) { return Err(McpError::InvalidConfig("reserved header".into())); }
                    let mut value = HeaderValue::from_str(&value).map_err(|_| McpError::CredentialUnavailable)?;
                    value.set_sensitive(true); values.insert(name, value);
                }
                let client = Client::builder().redirect(reqwest::redirect::Policy::none()).default_headers(values).timeout(Duration::from_secs(15)).build().map_err(|_| McpError::ConnectionFailed("HTTP client unavailable".into()))?;
                Ok(Self { session: Session::probe_http(url, client)?, legacy_sse, container: None, secrets })
            }
        }
    }
    pub async fn read_tools(&self, mut phase: impl FnMut(&'static str)) -> Result<ProbeTools> {
        phase("handshake");
        if self.legacy_sse { self.session.select_legacy_sse().await?; }
        let result = protocol::ensure_response(self.session.request(&protocol::request(crate::transport::next_id(), "initialize", protocol::initialize_params()), 15_000).await?)?;
        let protocol_version = result.get("protocolVersion").and_then(Value::as_str).unwrap_or_default().to_owned();
        self.session.complete_initialize(result)?;
        if let Some(container) = &self.container { container.acknowledge_start(); }
        self.session.notify(&JsonRpcNotification { jsonrpc: "2.0", method: "notifications/initialized".into(), params: json!({}) }).await?;
        let mut tools = Vec::new(); let mut names = HashSet::new(); let mut cursors = HashSet::new(); let mut cursor = None;
        let mut budget = SchemaBudget(0);
        phase("tools");
        if self.session.has_tools()? {
            loop {
                let params = cursor.as_ref().map_or_else(|| json!({}), |cursor| json!({"cursor": cursor}));
                let value = protocol::ensure_response(self.session.request(&protocol::request(crate::transport::next_id(), "tools/list", params), 15_000).await?)?;
                if contains_credentials(&value, &self.secrets) { return Err(McpError::Protocol("server schema echoed host credentials".into())); }
                let page = value.get("tools").and_then(Value::as_array).ok_or_else(|| McpError::Protocol("invalid tools list".into()))?;
                serde_json::to_writer(&mut budget, &value).map_err(|_| McpError::Protocol("complete schemas exceed limit".into()))?;
                for tool in page {
                    let name = tool.get("name").and_then(Value::as_str).filter(|name| !name.is_empty() && name.len() <= 256).ok_or_else(|| McpError::Protocol("invalid tool name".into()))?;
                    if !names.insert(name.to_owned()) || tool.pointer("/inputSchema/type").and_then(Value::as_str) != Some("object") || tool.get("outputSchema").is_some_and(|schema| schema.get("type").and_then(Value::as_str) != Some("object")) { return Err(McpError::Protocol("incomplete tool schema".into())); }
                    if tools.len() >= 2048 { return Err(McpError::Protocol("tool limit exceeded".into())); }
                    tools.push(tool.clone());
                }
                cursor = match value.get("nextCursor") { None | Some(Value::Null) => None, Some(Value::String(value)) if !value.is_empty() && value.len() <= 4096 => Some(value.clone()), _ => return Err(McpError::Protocol("invalid tools cursor".into())) };
                let Some(next) = &cursor else { break; };
                if !cursors.insert(next.clone()) || cursors.len() > 64 { return Err(McpError::Protocol("tools pagination did not terminate".into())); }
            }
        }
        Ok(ProbeTools { protocol_version, tools })
    }
    pub async fn cleanup(&self) -> bool {
        let (process, container) = tokio::join!(self.session.close_confirmed(), async { if let Some(container) = &self.container { container.cleanup().await } else { true } });
        process && container
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_shell_and_reserved_credential_headers() {
        assert!(ProbeConnection::connect(ProbeEndpoint::Stdio { executable: "/bin/sh".into(), args: vec![], cwd: "/tmp".into(), environment: vec![], container: None }).is_err());
        assert!(ProbeConnection::connect(ProbeEndpoint::Http { url: "https://example.com/mcp".into(), headers: vec![("Mcp-Session-Id".into(), "secret".into())], legacy_sse: false }).is_err());
    }
}

struct SchemaBudget(usize);
impl std::io::Write for SchemaBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        if self.0 > 4 * 1024 * 1024 { return Err(std::io::Error::other("schema limit")); }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod probe_tests;

fn contains_credentials(value: &Value, secrets: &[zeroize::Zeroizing<String>]) -> bool {
    let sensitive = |text: &str| secrets.iter().any(|secret| text == secret.as_str() || secret.len() >= 4 && text.contains(secret.as_str()));
    match value {
        Value::String(text) => sensitive(text),
        Value::Array(values) => values.iter().any(|value| contains_credentials(value, secrets)),
        Value::Object(values) => values.iter().any(|(key,value)| sensitive(key) || contains_credentials(value, secrets)),
        Value::Number(number) => { let text = number.to_string(); secrets.iter().any(|secret| secret.len() >= 4 && text.contains(secret.as_str())) },
        _ => false,
    }
}

#[cfg(test)]
mod credential_echo_tests {
    use super::*;
    #[test]
    fn rejects_host_credentials_embedded_in_schema_text_or_keys() {
        let secrets = vec![zeroize::Zeroizing::new("native-private-token".into())];
        assert!(contains_credentials(&json!({"tools":[{"name":"test","description":"echo native-private-token"}]}), &secrets));
        assert!(contains_credentials(&json!({"native-private-token":{"type":"string"}}), &secrets));
        assert!(!contains_credentials(&json!({"inputSchema":{"type":"object"}}), &secrets));
        let numeric = vec![zeroize::Zeroizing::new("123456789".into())];
        assert!(contains_credentials(&json!({"default":123456789}), &numeric));
    }
}
