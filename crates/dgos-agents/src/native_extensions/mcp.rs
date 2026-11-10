//! Native MCP format adapter. Reads never connect; edits expose no raw values.
use super::{config::{self, Patch}, Error, Owner, Scope, Target};
use serde::Serialize;
use serde_json::{json, Value};
#[path = "auth.rs"]
pub mod auth;

#[derive(Clone, Debug, Serialize)]
pub struct Instance {
    pub native_key: String,
    pub transport: Transport,
    pub enabled: Option<bool>,
    pub has_auth_configuration: bool,
    pub unknown_fields_present: bool,
    pub owner: Owner,
    pub authentication: auth::Projection,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Transport { Stdio, Http, Sse, WebSocket, Unknown }
#[derive(Clone, Debug, Serialize)]
pub struct Discovery { pub instances: Vec<Instance>, pub incomplete: bool }

/// Keep/Set/Remove is required: a masked value must never round-trip to disk.
#[derive(Clone)]
pub enum Field<T> { Keep, Set(T), Remove }
impl<T> Default for Field<T> { fn default() -> Self { Self::Keep } }
#[derive(Clone, Default)]
pub struct ServerPatch {
    pub transport: Field<Transport>,
    pub program: Field<String>,
    pub args: Field<Vec<String>>,
    pub url: Field<String>,
    pub cwd: Field<String>,
    pub bearer_env: Field<String>,
    pub forwarded_env: Field<Vec<String>>,
    pub header_env: Field<Vec<(String, String)>>,
}
#[derive(Clone)]
pub enum Change {
    Upsert { name: String, fields: ServerPatch },
    Remove { name: String },
    SetEnabled { name: String, enabled: bool },
}
fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}
fn env_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().enumerate().all(|(i,b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
fn base(scope: &Scope, project: Option<&str>) -> Result<Vec<String>, Error> {
    Ok(if *scope == Scope::ProjectLocal { vec!["projects".into(), project.ok_or(Error::AccessDenied)?.into(), "mcpServers".into()] } else { vec!["mcpServers".into()] })
}
fn push<T>(patches: &mut Vec<Patch>, path: &[String], key: &str, field: Field<T>, convert: impl FnOnce(T) -> Result<Value, Error>) -> Result<(), Error> {
    let value = match field { Field::Keep => return Ok(()), Field::Set(value) => Some(convert(value)?), Field::Remove => None };
    let mut path = path.to_vec(); path.push(key.into()); patches.push((path, value)); Ok(())
}
fn safe_text(value: &str) -> Result<(), Error> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) { return Err(Error::InvalidFormat); }
    let lower = value.to_ascii_lowercase();
    if ["****", "[redacted]", "<redacted>", "<masked>", "sk-", "ghp_", "github_pat_", "xoxb-", "xoxp-", "bearer ", "authorization", "password=", "password:", "token=", "token:", "secret=", "secret:", "api_key=", "api-key=", "api-key:"].iter().any(|s| lower.contains(*s)) {
        return Err(Error::InvalidFormat);
    }
    Ok(())
}
fn string(value: String) -> Result<Value, Error> {
    safe_text(&value)?;
    Ok(Value::String(value))
}
fn program(value: String) -> Result<Value, Error> {
    safe_text(&value)?;
    if value.chars().any(char::is_whitespace) || value.contains([';', '|', '&', '`', '$', '>', '<']) { return Err(Error::InvalidFormat); }
    Ok(Value::String(value))
}
fn arguments(values: Vec<String>) -> Result<Value, Error> {
    if values.len() > 256 || values.iter().map(String::len).sum::<usize>() > 32 * 1024 { return Err(Error::Limit); }
    for value in &values {
        // Empty argv entries are legitimate; credential-bearing switches are not.
        if value.is_empty() { continue; }
        safe_text(value)?;
        let key = value.trim_start_matches('-').split('=').next().unwrap_or("").to_ascii_lowercase();
        if key.split(['-', '_']).any(|part| matches!(part, "token" | "password" | "secret")) || ["api-key", "api_key", "apikey", "accesstoken", "clientsecret", "header", "headers", "h", "u"].contains(&key.as_str()) {
            return Err(Error::InvalidFormat);
        }
        if value.contains("://") { url(value.clone())?; }
    }
    Ok(json!(values))
}
fn url(value: String) -> Result<Value, Error> {
    safe_text(&value)?;
    if !value.is_ascii() || value.contains(['@', '?', '#', '\\']) || value.bytes().any(|b| b.is_ascii_whitespace()) { return Err(Error::InvalidFormat); }
    let uri = value.parse::<axum::http::Uri>().map_err(|_| Error::InvalidFormat)?;
    let authority = uri.authority().ok_or(Error::InvalidFormat)?;
    let host = authority.host();
    let scheme = uri.scheme_str().ok_or(Error::InvalidFormat)?;
    if host.is_empty() || host.contains('%') || !(scheme == "https" || (scheme == "http" && matches!(host, "127.0.0.1" | "localhost" | "[::1]"))) {
        return Err(Error::InvalidFormat);
    }
    if authority.as_str().ends_with(':') || (authority.port().is_some() && authority.port_u16().is_none()) { return Err(Error::InvalidFormat); }
    Ok(Value::String(value))
}
pub(super) fn patches(target: Target, scope: &Scope, project: Option<&str>, change: Change) -> Result<Vec<Patch>, Error> {
    let mut path = if target == Target::Codex {
        if *scope == Scope::ProjectLocal { return Err(Error::Unsupported); }
        vec!["mcp_servers".into()]
    } else { base(scope, project)? };
    let name = match &change { Change::Upsert { name, .. } | Change::Remove { name } | Change::SetEnabled { name, .. } => name };
    if !valid_name(name) { return Err(Error::InvalidFormat); }
    path.push(name.clone());
    let mut output = Vec::new();
    match change {
        Change::Remove { .. } => output.push((path, None)),
        Change::SetEnabled { enabled, .. } => {
            if target != Target::Codex { return Err(Error::Unsupported); }
            path.push("enabled".into()); output.push((path, Some(json!(enabled))));
        }
        Change::Upsert { fields, .. } => {
            let ServerPatch { transport, program, args, url: endpoint, cwd, bearer_env, forwarded_env, header_env } = fields;
            if target == Target::Claude {
                if !matches!(cwd, Field::Keep) || !matches!(bearer_env, Field::Keep) || !matches!(forwarded_env, Field::Keep) { return Err(Error::Unsupported); }
                push(&mut output, &path, "type", transport, |t| Ok(json!(match t { Transport::Stdio => "stdio", Transport::Http => "http", Transport::Sse => "sse", Transport::WebSocket => return Err(Error::Unsupported), Transport::Unknown => return Err(Error::InvalidFormat) })))?;
            } else if !matches!(transport, Field::Keep) {
                // Codex discriminates by command vs URL, not by a type field.
                // Changing transport removes only its mutually exclusive known
                // endpoint fields; auth and unknown fields still remain.
                match transport {
                    Field::Set(Transport::Stdio) => { let mut p = path.clone(); p.push("url".into()); output.push((p, None)); }
                    Field::Set(Transport::Http) => {
                        for key in ["command", "args", "cwd"] { let mut p = path.clone(); p.push(key.into()); output.push((p,None)); }
                    }
                    _ => return Err(Error::Unsupported),
                }
            }
            push(&mut output, &path, "command", program, self::program)?;
            push(&mut output, &path, "args", args, arguments)?;
            push(&mut output, &path, "url", endpoint, url)?;
            if target == Target::Codex {
                push(&mut output, &path, "cwd", cwd, string)?;
                push(&mut output, &path, "bearer_token_env_var", bearer_env, |v| if env_name(&v) { Ok(json!(v)) } else { Err(Error::InvalidFormat) })?;
                push(&mut output, &path, "env_vars", forwarded_env, |values| {
                    let mut seen = std::collections::HashSet::new();
                    if values.len() > 128 || !values.iter().all(|v| env_name(v) && seen.insert(v.as_str())) { return Err(Error::InvalidFormat); }
                    Ok(json!(values))
                })?;
                patch_headers(&mut output, &path, "env_http_headers", header_env, false)?;
            } else { patch_headers(&mut output, &path, "headers", header_env, true)?; }
            if output.is_empty() { return Err(Error::InvalidFormat); }
        }
    }
    Ok(output)
}
fn patch_headers(patches: &mut Vec<Patch>, path: &[String], key: &str, field: Field<Vec<(String,String)>>, expand: bool) -> Result<(), Error> {
    let values = match field {
        Field::Keep => return Ok(()),
        Field::Remove if expand => return Err(Error::Unsupported),
        Field::Remove => { let mut path = path.to_vec(); path.push(key.into()); patches.push((path, None)); return Ok(()); }
        Field::Set(values) => values,
    };
    if values.len() > 64 { return Err(Error::Limit); }
    let mut seen = std::collections::HashSet::new();
    for (header, variable) in values {
        if header.is_empty() || header.len() > 128 || !header.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) || !env_name(&variable) || !seen.insert(header.to_ascii_lowercase()) { return Err(Error::InvalidFormat); }
        let value = if expand { format!("${{{variable}}}") } else { variable };
        let mut path = path.to_vec(); path.extend([key.into(), header]);
        patches.push((path, Some(json!(value))));
    }
    Ok(())
}
fn marked_owner<'a>(name: &str, markers: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> Owner {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("plugin:") { return Owner::Plugin; }
    if lower.starts_with("managed:") { return Owner::Managed; }
    if lower.starts_with("synced:") { return Owner::Synced; }
    for (key, value) in markers {
        let key = key.to_ascii_lowercase();
        if matches!(key.as_str(), "plugin" | "plugin_id" | "pluginid" | "plugin_name" | "pluginname" | "plugin_source" | "pluginsource") { return Owner::Plugin; }
        if matches!(key.as_str(), "managed" | "managed_by" | "managedby" | "managedpolicy" | "managed_policy") { return Owner::Managed; }
        if matches!(key.as_str(), "synced" | "cloud_synced" | "cloudsynced" | "connector" | "sync_source" | "syncsource") { return Owner::Synced; }
        if matches!(key.as_str(), "owner" | "source" | "ownership") {
            return match value.map(str::to_ascii_lowercase).as_deref() {
                Some("plugin") => Owner::Plugin,
                Some("managed" | "policy") => Owner::Managed,
                Some("synced" | "connector" | "cloud") => Owner::Synced,
                _ => Owner::Unknown,
            };
        }
    }
    Owner::Loose
}
fn json_owner(name: &str, nodes: &[&Value]) -> Owner {
    marked_owner(name, nodes.iter().filter_map(|v| v.as_object()).flat_map(|o| o.iter().map(|(k,v)| (k.as_str(), v.as_str()))))
}
fn toml_owner(name: &str, nodes: &[&dyn toml_edit::TableLike]) -> Owner {
    marked_owner(name, nodes.iter().flat_map(|t| t.iter().map(|(k,v)| (k, v.as_str()))))
}
fn change_name(change: &Change) -> &str {
    match change { Change::Upsert { name, .. } | Change::Remove { name } | Change::SetEnabled { name, .. } => name }
}
/// Called with the exact locked preimage, before any deletion or field patch.
pub(super) fn validate_original(text: &str, target: Target, scope: &Scope, project: Option<&str>, change: &Change) -> Result<(), Error> {
    let discovery = discover(text, target, scope, project)?;
    if discovery.incomplete { return Err(Error::Limit); }
    let name = change_name(change);
    if marked_owner(name, std::iter::empty()) != Owner::Loose { return Err(Error::Unsupported); }
    match discovery.instances.iter().find(|v| v.native_key == name) {
        Some(instance) if instance.owner != Owner::Loose || matches!(instance.transport, Transport::Unknown | Transport::WebSocket) => Err(Error::Unsupported),
        Some(_) => validate_existing_headers(text, target, scope, project, change),
        None if !matches!(change, Change::Upsert { .. }) => Err(Error::NotFound),
        None => {
            // Missing instances must not bypass enclosing ownership markers.
            let owner = match target {
                Target::Claude if !text.is_empty() => {
                    let root = config::json_read(text)?;
                    let mut nodes = vec![&root];
                    let mut node = &root;
                    let path = base(scope, project)?;
                    for (index, key) in path.iter().enumerate() {
                        match node.get(key) {
                            Some(next) => { node = next; if index + 1 < path.len() { nodes.push(node); } },
                            None => break,
                        }
                    }
                    json_owner(name, &nodes)
                }
                Target::Codex => {
                    let doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
                    toml_owner(name, &[doc.as_table()])
                }
                _ => Owner::Loose,
            };
            if owner != Owner::Loose { Err(Error::Unsupported) } else { Ok(()) }
        }
    }
}
fn validate_existing_headers(text: &str, target: Target, scope: &Scope, project: Option<&str>, change: &Change) -> Result<(), Error> {
    let Change::Upsert { name, fields } = change else { return Ok(()); };
    let Field::Set(headers) = &fields.header_env else { return Ok(()); };
    let existing: Vec<String> = match target {
        Target::Claude => {
            let root = config::json_read(text)?;
            let mut node = &root;
            for key in base(scope, project)? { node = node.get(&key).ok_or(Error::InvalidFormat)?; }
            match node.get(name).and_then(|v| v.get("headers")) {
                None => Vec::new(),
                Some(value) => value.as_object().ok_or(Error::InvalidFormat)?.keys().cloned().collect(),
            }
        }
        Target::Codex => {
            let doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
            let table = doc.get("mcp_servers").and_then(|v| v.get(name)).and_then(toml_edit::Item::as_table_like).ok_or(Error::InvalidFormat)?;
            let mut names = Vec::new();
            for key in ["http_headers", "env_http_headers"] {
                if let Some(value) = table.get(key) {
                    for (name, _) in value.as_table_like().ok_or(Error::InvalidFormat)?.iter() {
                        if key == "http_headers" && headers.iter().any(|(h,_)| h.eq_ignore_ascii_case(name)) { return Err(Error::Conflict); }
                        names.push(name.to_owned());
                    }
                }
            }
            names
        }
    };
    let mut seen = std::collections::HashSet::new();
    for name in &existing { if !seen.insert(name.to_ascii_lowercase()) { return Err(Error::InvalidFormat); } }
    for (name, _) in headers {
        if existing.iter().any(|key| key.eq_ignore_ascii_case(name) && key != name) { return Err(Error::Conflict); }
    }
    Ok(())
}
pub(super) fn validate_edited(text: &str, target: Target, scope: &Scope, project: Option<&str>, change: &Change) -> Result<(), Error> {
    let discovery = discover(text, target, scope, project)?;
    if discovery.incomplete { return Err(Error::Limit); }
    if let Change::Upsert { name, .. } | Change::SetEnabled { name, .. } = change {
        let instance = discovery.instances.iter().find(|v| &v.native_key == name).ok_or(Error::InvalidFormat)?;
        if instance.owner != Owner::Loose { return Err(Error::Unsupported); }
        if instance.transport == Transport::Unknown { return Err(Error::InvalidFormat); }
        validate_endpoint_fields(text, target, scope, project, change_name(change), instance.transport)?;
    }
    Ok(())
}
fn validate_endpoint_fields(text: &str, target: Target, scope: &Scope, project: Option<&str>, name: &str, transport: Transport) -> Result<(), Error> {
    let check = |has: &dyn Fn(&str) -> bool, args_ok: bool| {
        let incompatible = match transport {
            Transport::Stdio => ["url", "headers", "headersHelper", "http_headers", "env_http_headers", "bearer_token_env_var", "oauth"].iter().any(|k| has(k)),
            Transport::Http | Transport::Sse => ["command", "args", "cwd", "env", "env_vars"].iter().any(|k| has(k)),
            _ => true,
        };
        if incompatible || !args_ok { Err(Error::InvalidFormat) } else { Ok(()) }
    };
    match target {
        Target::Claude => {
            let root = config::json_read(text)?;
            let mut node = &root;
            for key in base(scope, project)? { node = node.get(&key).ok_or(Error::InvalidFormat)?; }
            let object = node.get(name).and_then(Value::as_object).ok_or(Error::InvalidFormat)?;
            let args_ok = object.get("args").is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(Value::is_string)));
            check(&|key| object.contains_key(key), args_ok)
        }
        Target::Codex => {
            let doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
            let table = doc.get("mcp_servers").and_then(|s| s.get(name)).and_then(toml_edit::Item::as_table_like).ok_or(Error::InvalidFormat)?;
            let args_ok = table.get("args").is_none_or(|v| v.as_array().is_some_and(|a| a.iter().all(|v| v.as_str().is_some())));
            if table.get("enabled").is_some_and(|v| v.as_bool().is_none()) { return Err(Error::InvalidFormat); }
            check(&|key| table.contains_key(key), args_ok)
        }
    }
}
pub(super) fn discover(text: &str, target: Target, scope: &Scope, project: Option<&str>) -> Result<Discovery, Error> {
    let mut instances = Vec::new(); let mut incomplete = false;
    if text.is_empty() { return Ok(Discovery { instances, incomplete }); }
    if text.len() > config::CONFIG_LIMIT { return Err(Error::Limit); }
    match target {
        Target::Claude => {
            let value = config::json_read(text)?;
            let mut node = &value;
            let mut owner_nodes = vec![&value];
            let path = base(scope, project)?;
            for (index, key) in path.iter().enumerate() {
                match node.get(key) {
                    Some(next) => { node = next; if index + 1 < path.len() { owner_nodes.push(node); } },
                    None => return Ok(Discovery { instances, incomplete }),
                }
            }
            let servers = node.as_object().ok_or(Error::InvalidFormat)?;
            for (name, server) in servers.iter().take(2049) {
                if instances.len() == 2048 { incomplete = true; break; }
                if !valid_name(name) { incomplete = true; continue; }
                let object = server.as_object().ok_or(Error::InvalidFormat)?;
                let kind = match object.get("type") { None => "stdio", Some(value) => value.as_str().unwrap_or("") };
                let transport = match kind {
                    "stdio" if object.get("command").and_then(Value::as_str).is_some() && !object.contains_key("url") => Transport::Stdio,
                    "http" | "streamable-http" if object.get("url").and_then(Value::as_str).is_some() && !object.contains_key("command") => Transport::Http,
                    "sse" if object.get("url").and_then(Value::as_str).is_some() && !object.contains_key("command") => Transport::Sse,
                    "ws" if object.get("url").and_then(Value::as_str).is_some() && !object.contains_key("command") => Transport::WebSocket,
                    _ => Transport::Unknown,
                };
                let mut nodes = owner_nodes.clone(); nodes.push(server);
                let owner = json_owner(name, &nodes);
                let has_auth_configuration = object.keys().any(|k| matches!(k.as_str(), "headers" | "headersHelper" | "oauth" | "env"));
                instances.push(Instance { native_key: name.clone(), transport, enabled: None, has_auth_configuration, owner, authentication: auth::Projection::configuration_only(has_auth_configuration), unknown_fields_present: object.keys().any(|k| !["type", "command", "args", "url", "headers", "env"].contains(&k.as_str())) });
            }
        }
        Target::Codex => {
            let doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
            if let Some(servers) = doc.get("mcp_servers") {
                let servers = servers.as_table_like().ok_or(Error::InvalidFormat)?;
                for (name, server) in servers.iter().take(2049) {
                    if instances.len() == 2048 { incomplete = true; break; }
                    if !valid_name(name) { incomplete = true; continue; }
                    let table = server.as_table_like().ok_or(Error::InvalidFormat)?;
                    let transport = match (table.get("command").and_then(toml_edit::Item::as_str), table.get("url").and_then(toml_edit::Item::as_str)) {
                        (Some(_), None) if !table.contains_key("url") && !table.contains_key("type") => Transport::Stdio,
                        (None, Some(_)) if !table.contains_key("command") && !table.contains_key("type") => Transport::Http,
                        _ => Transport::Unknown,
                    };
                    let owner = toml_owner(name, &[doc.as_table(), table]);
                    let has_auth_configuration = ["env", "env_vars", "bearer_token_env_var", "http_headers", "env_http_headers", "oauth"].iter().any(|k| table.contains_key(k));
                    instances.push(Instance { native_key: name.into(), transport, enabled: table.get("enabled").and_then(toml_edit::Item::as_bool), has_auth_configuration, owner, authentication: auth::Projection::configuration_only(has_auth_configuration), unknown_fields_present: table.iter().any(|(k,_)| !["command", "args", "url", "cwd", "enabled", "env_vars", "bearer_token_env_var", "env_http_headers"].contains(&k)) });
                }
            }
        }
    }
    Ok(Discovery { instances, incomplete })
}
