use crate::error::{McpError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PREFERRED_PROTOCOL_VERSION: &str = "2025-06-18";
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub struct NegotiatedProtocol {
    pub version: String,
    pub capabilities: serde_json::Map<String, Value>,
    _server_info: Value,
}

impl NegotiatedProtocol {
    pub fn from_initialize(value: Value) -> Result<Self> {
        let version = value
            .get("protocolVersion")
            .and_then(Value::as_str)
            .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
            .ok_or_else(|| McpError::HandshakeFailed("unsupported protocol version".into()))?;
        let capabilities = value
            .get("capabilities")
            .and_then(Value::as_object)
            .ok_or_else(|| McpError::HandshakeFailed("missing server capabilities".into()))?;
        if capabilities.get("tools").is_some_and(|tools| {
            !tools.is_object()
                || tools
                    .get("listChanged")
                    .is_some_and(|changed| !changed.is_boolean())
        }) {
            return Err(McpError::HandshakeFailed("invalid tools capability".into()));
        }
        let info = value.get("serverInfo");
        if ["name", "version"].iter().any(|field| {
            info.and_then(|info| info.get(field))
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        }) {
            return Err(McpError::HandshakeFailed(
                "invalid server information".into(),
            ));
        }
        Ok(Self {
            version: version.to_owned(),
            capabilities: capabilities.clone(),
            _server_info: value["serverInfo"].clone(),
        })
    }
}

// The client advertises no sampling, roots, elicitation or task capabilities.
// Server metadata and requests cannot grant themselves host permissions.
pub fn server_request_reply(value: &Value) -> Result<Option<Value>> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpError::Protocol("invalid JSON-RPC version".into()));
    }
    let Some(method) = value.get("method") else {
        return Ok(None);
    };
    let method = method
        .as_str()
        .ok_or_else(|| McpError::Protocol("invalid JSON-RPC method".into()))?;
    let Some(id) = value.get("id") else {
        return Ok(None);
    };
    if !(id.is_string() || id.is_i64() || id.is_u64()) {
        return Err(McpError::Protocol("invalid server request id".into()));
    }
    Ok(Some(if method == "ping" {
        json!({"jsonrpc": "2.0", "id": id, "result": {}})
    } else {
        json!({"jsonrpc": "2.0", "id": id, "error": {
            "code": -32601, "message": "Client method not supported"
        }})
    }))
}

#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(rename = "data")]
    pub _data: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(rename = "outputSchema")]
    pub output_schema: Option<Value>,
}

pub fn request(id: u64, method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0",
        id,
        method: method.to_string(),
        params,
    }
}

pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": PREFERRED_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "DGOS", "version": "1.0.0" }
    })
}

pub fn ensure_response(response: JsonRpcResponse) -> Result<Value> {
    if let Some(error) = response.error {
        // Server messages and error data can contain secrets or full tool arguments.
        let _ = error.message;
        return Err(McpError::ServerReturned(format!(
            "JSON-RPC code {}",
            error.code
        )));
    }
    response
        .result
        .ok_or_else(|| McpError::Protocol("response did not contain result".into()))
}

pub fn parse_tools(value: Value) -> Result<Vec<ToolDefinition>> {
    let tools = value
        .get("tools")
        .cloned()
        .ok_or_else(|| McpError::Protocol("tools/list response has no tools".into()))?;
    serde_json::from_value(tools).map_err(|e| McpError::Protocol(e.to_string()))
}

pub struct ToolCallResult {
    pub content: Value,
    pub structured_content: Option<Value>,
    pub is_error: bool,
}

pub fn parse_call_result(value: Value, output_schema: Option<&str>) -> Result<ToolCallResult> {
    // Never expose validation diagnostics containing the server's result data.
    let invalid = || McpError::Protocol("invalid tool result contract".into());
    if !value.is_object() || value.get("isError").is_some_and(|v| !v.is_boolean()) {
        return Err(invalid());
    }
    let content = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let has_strings = |item: &Value, names: &[&str]| {
        names
            .iter()
            .all(|name| item.get(*name).is_some_and(Value::is_string))
    };
    for item in content {
        let valid = match item.get("type").and_then(Value::as_str) {
            Some("text") => has_strings(item, &["text"]),
            Some("image" | "audio") => has_strings(item, &["data", "mimeType"]),
            Some("resource_link") => has_strings(item, &["name", "uri"]),
            Some("resource") => item.get("resource").is_some_and(|resource| {
                has_strings(resource, &["uri"])
                    && (has_strings(resource, &["text"]) || has_strings(resource, &["blob"]))
                    && resource.get("mimeType").is_none_or(Value::is_string)
            }),
            _ => false,
        };
        if !valid
            || item.get("annotations").is_some_and(|v| !v.is_object())
            || item.get("_meta").is_some_and(|v| !v.is_object())
        {
            return Err(invalid());
        }
    }
    let structured = value.get("structuredContent");
    if structured.is_some_and(|v| !v.is_object()) {
        return Err(invalid());
    }
    let is_error = value
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(schema) = output_schema.filter(|_| !is_error) {
        let schema: Value = serde_json::from_str(schema).map_err(|_| invalid())?;
        let validator = jsonschema::validator_for(&schema).map_err(|_| invalid())?;
        if !structured.is_some_and(|result| validator.is_valid(result)) {
            return Err(invalid());
        }
    }
    Ok(ToolCallResult {
        content: Value::Array(content.clone()),
        structured_content: structured.cloned(),
        is_error,
    })
}

#[cfg(test)]
mod result_tests {
    use super::*;

    #[test]
    fn validates_result_envelope_content_and_optional_output_schema_without_disclosing_data() {
        let schema =
            json!({"type":"object","required":["text"],"properties":{"text":{"type":"string"}}})
                .to_string();
        let valid = json!({"content":[{"type":"text","text":"private-result","extension":true}],"structuredContent":{"text":"value"}});
        assert!(parse_call_result(valid.clone(), Some(&schema)).is_ok());
        for invalid in [
            json!({}),
            json!({"content":"private-result"}),
            json!({"content":[],"isError":"false"}),
            json!({"content":[{"type":"text","text":123}]}),
            json!({"content":[{"type":"image","data":"data"}]}),
            json!({"content":[{"type":"resource","resource":{"uri":"file:///private"}}]}),
            json!({"content":[],"structuredContent":null}),
        ] {
            let error = parse_call_result(invalid, None).err().unwrap();
            assert_eq!(error.class(), "mcp_protocol_error");
            assert!(!error.to_string().contains("private"));
        }
        assert!(parse_call_result(json!({"content":[]}), Some(&schema)).is_err());
        assert!(parse_call_result(
            json!({"content":[],"structuredContent":{"text":1}}),
            Some(&schema)
        )
        .is_err());
        assert!(parse_call_result(json!({"content":[],"isError":true}), Some(&schema)).is_ok());
        assert!(parse_call_result(
            json!({"content":[
                {"type":"image","data":"AA==","mimeType":"image/png"},
                {"type":"audio","data":"AA==","mimeType":"audio/wav"},
                {"type":"resource_link","uri":"https://example.invalid/resource","name":"resource"},
                {"type":"resource","resource":{"uri":"file:///fixture","text":"data"}},
                {"type":"resource","resource":{"uri":"file:///fixture","blob":"AA=="}}
            ]}),
            None
        )
        .is_ok());
    }
}
