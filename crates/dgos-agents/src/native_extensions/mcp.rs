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
                if !matches!(cwd, Field::Keep) || !matches!(bearer_env, Field::Keep) { return Err(Error::Unsupported); }
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
            } else {
                push(&mut output, &path, "env", forwarded_env, |values| {
                    let mut map = serde_json::Map::new();
                    if values.len() > 128 { return Err(Error::Limit); }
                    for name in values { if !env_name(&name) || map.contains_key(&name) { return Err(Error::InvalidFormat); } map.insert(name.clone(), json!(format!("${{{name}}}"))); }
                    Ok(Value::Object(map))
                })?;
                patch_headers(&mut output, &path, "headers", header_env, true)?;
            }
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

/// This host-only preimage must never be serialized or logged. Literal native
/// credentials stay here; the renderer sees only their field/reference names.
#[derive(Clone)]
pub struct ProbeConfig {
    pub projection: ConfigProjection,
    pub environment: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub forwarded_env: Vec<String>,
    pub header_env: Vec<(String, String)>,
    pub bearer_env: Option<String>,
    pub credential_supported: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConfigProjection {
    pub name: String, pub transport: Transport, pub program: Option<String>,
    pub args: Vec<String>, pub url: Option<String>, pub cwd: Option<String>,
    pub credential_references: Vec<String>, pub redacted: bool, pub config_digest: String,
}
fn text_list(value: Option<&Value>) -> Result<Vec<String>, Error> {
    match value { None => Ok(Vec::new()), Some(value) => value.as_array().ok_or(Error::InvalidFormat)?.iter().map(|v| v.as_str().map(str::to_owned).ok_or(Error::InvalidFormat)).collect() }
}
fn pairs(value: Option<&Value>) -> Result<Vec<(String, String)>, Error> {
    match value { None => Ok(Vec::new()), Some(value) => value.as_object().ok_or(Error::InvalidFormat)?.iter().map(|(k,v)| Ok((k.clone(), v.as_str().ok_or(Error::InvalidFormat)?.to_owned()))).collect() }
}
fn reference(value: &str) -> Option<String> {
    let name = value.strip_prefix("${")?.strip_suffix('}')?;
    env_name(name).then(|| name.to_owned())
}
fn probe_node(node: &Value, name: &str, transport: Transport, digest: String, target: Target) -> Result<ProbeConfig, Error> {
    let get = |key: &str| -> Result<Option<String>, Error> { node.get(key).map(|v| v.as_str().map(str::to_owned).ok_or(Error::InvalidFormat)).transpose() };
    let raw_program = get("command")?; let raw_url = get("url")?; let raw_cwd = get("cwd")?; let raw_args = text_list(node.get("args"))?;
    let safe_program = raw_program.as_ref().filter(|v| program((*v).clone()).is_ok()).cloned();
    let safe_url = raw_url.as_ref().filter(|v| url((*v).clone()).is_ok()).cloned();
    let safe_cwd = raw_cwd.as_ref().filter(|v| safe_text(v).is_ok()).cloned();
    let safe_args = arguments(raw_args.clone()).is_ok() && !raw_args.iter().any(|argument| argument.contains("${"));
    let redacted = raw_program != safe_program || raw_url != safe_url || raw_cwd != safe_cwd || !safe_args;
    let environment = pairs(node.get("env"))?;
    if environment.len() > 128 || environment.iter().any(|(name, _)| !env_name(name)) { return Err(Error::InvalidFormat); }
    let mut headers = Vec::new(); let mut header_env = Vec::new();
    let forwarded_env = text_list(node.get("env_vars"))?;
    if forwarded_env.len() > 128 || forwarded_env.iter().any(|name| !env_name(name)) { return Err(Error::InvalidFormat); }
    let bearer_env = get("bearer_token_env_var")?;
    if bearer_env.as_ref().is_some_and(|name| !env_name(name)) { return Err(Error::InvalidFormat); }
    for (name, value) in pairs(node.get(if target == Target::Codex { "env_http_headers" } else { "headers" }))? {
        if target == Target::Codex { if !env_name(&value) { return Err(Error::InvalidFormat); } header_env.push((name, value)); }
        else if let Some(variable) = reference(&value) { header_env.push((name, variable)); }
        else { headers.push((name, value)); }
    }
    headers.extend(pairs(node.get("http_headers"))?);
    if headers.len() + header_env.len() > 64 || headers.iter().map(|(name,_)| name).chain(header_env.iter().map(|(name,_)| name)).any(|name| name.is_empty() || name.len() > 128 || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&byte))) { return Err(Error::InvalidFormat); }
    let mut header_names = std::collections::HashSet::new();
    for (name, _) in headers.iter().chain(header_env.iter()) { if !header_names.insert(name.to_ascii_lowercase()) { return Err(Error::InvalidFormat); } }
    if bearer_env.is_some() && header_names.contains("authorization") { return Err(Error::InvalidFormat); }
    let mut credential_references: Vec<String> = environment.iter().map(|(name,value)| (if target == Target::Claude { reference(value) } else { None }).map_or_else(|| format!("native env:{name}"), |variable| format!("env:{variable}"))).collect();
    credential_references.extend(forwarded_env.iter().map(|name| format!("env:{name}")));
    credential_references.extend(header_env.iter().map(|(header,name)| format!("header:{header} ← env:{name}")));
    credential_references.extend(headers.iter().map(|(name,_)| format!("native header:{name}")));
    if let Some(name) = &bearer_env { credential_references.push(format!("bearer ← env:{name}")); }
    Ok(ProbeConfig { projection: ConfigProjection { name: name.into(), transport, program: safe_program, args: if safe_args { raw_args } else { Vec::new() }, url: safe_url, cwd: safe_cwd, credential_references, redacted, config_digest: digest }, environment, headers, forwarded_env, header_env, bearer_env, credential_supported: !["oauth", "headersHelper"].iter().any(|key| node.get(key).is_some()) })
}

/// Validates complete unsaved drafts using exactly the native writer's policy.
pub fn probe_draft(target: Target, scope: &Scope, name: String, fields: ServerPatch) -> Result<ProbeConfig, Error> {
    let change = Change::Upsert { name: name.clone(), fields };
    let patches = patches(target, scope, Some("/draft"), change.clone())?;
    let text = config::apply("", target, &patches)?;
    validate_edited(&text, target, scope, Some("/draft"), &change)?;
    let mut node = serde_json::Map::new();
    for (path, value) in patches { if let Some(value) = value { node.insert(path.last().ok_or(Error::InvalidFormat)?.clone(), value); } }
    let node = Value::Object(node);
    let transport = match node.get("type").and_then(Value::as_str) { Some("sse") => Transport::Sse, Some("http") => Transport::Http, Some("stdio") => Transport::Stdio, _ if node.get("command").is_some() => Transport::Stdio, _ if node.get("url").is_some() => Transport::Http, _ => return Err(Error::InvalidFormat) };
    #[cfg(unix)]
    { let digest = super::paths::hash(&serde_json::to_vec(&node).map_err(|_| Error::InvalidFormat)?); probe_node(&node, &name, transport, digest, target) }
    #[cfg(not(unix))]
    { let _ = (node, name, transport); Err(Error::Unsupported) }
}

impl super::NativeExtensions {
    pub fn probe_binding(&self, target: Target, scope: &Scope) -> Result<(), Error> {
        #[cfg(unix)] {
            self.validate(false)?;
            let path = self.context.config_path(target, scope)?;
            self.validate_path(path.parent().ok_or(Error::UnsafePath)?)
        }
        #[cfg(not(unix))] { let _ = (target, scope); Err(Error::Unsupported) }
    }
    pub fn probe_mcp_config(&self, target: Target, scope: Scope, name: &str) -> Result<ProbeConfig, Error> {
        #[cfg(unix)] {
            use super::paths::Directory;
            self.validate(false)?;
            if !valid_name(name) { return Err(Error::InvalidFormat); }
            let path = self.context.config_path(target, &scope)?;
            let parent = path.parent().ok_or(Error::UnsafePath)?;
            self.validate_path(parent)?;
            let directory = Directory::open(parent)?;
            let (text, snapshot) = directory.read(path.file_name().and_then(|v| v.to_str()).ok_or(Error::UnsafePath)?)?.ok_or(Error::NotFound)?;
            let discovery = discover(&text, target, &scope, self.context.roots.project.as_ref().and_then(|v| v.to_str()))?;
            let server = discovery.instances.iter().find(|server| server.native_key == name).ok_or(Error::NotFound)?;
            if !matches!(server.transport, Transport::Stdio | Transport::Http | Transport::Sse) { return Err(Error::Unsupported); }
            validate_endpoint_fields(&text, target, &scope, self.context.roots.project.as_ref().and_then(|v| v.to_str()), name, server.transport)?;
            let node = if target == Target::Claude {
                let document = config::json_read(&text)?; let mut node = &document;
                for key in base(&scope, self.context.roots.project.as_ref().and_then(|v| v.to_str()))? { node = node.get(&key).ok_or(Error::NotFound)?; }
                node.get(name).ok_or(Error::NotFound)?.clone()
            } else {
                let document = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
                let table = document.get("mcp_servers").and_then(|v| v.get(name)).and_then(toml_edit::Item::as_table_like).ok_or(Error::NotFound)?;
                let mut node = serde_json::Map::new();
                for (key,item) in table.iter() {
                    if !["command", "args", "url", "cwd", "env", "env_vars", "bearer_token_env_var", "env_http_headers", "http_headers", "oauth", "headersHelper"].contains(&key) { continue; }
                    if matches!(key, "oauth" | "headersHelper") { node.insert(key.into(), Value::Bool(true)); continue; }
                    if let Some(value) = item.as_str() { node.insert(key.into(), json!(value)); }
                    else if let Some(value) = item.as_array() { node.insert(key.into(), Value::Array(value.iter().map(|value| value.as_str().map(|v| json!(v)).ok_or(Error::InvalidFormat)).collect::<Result<_,_>>()?)); }
                    else if let Some(value) = item.as_table_like() { node.insert(key.into(), Value::Object(value.iter().map(|(k,v)| Ok((k.into(), json!(v.as_str().ok_or(Error::InvalidFormat)?)))).collect::<Result<_,Error>>()?)); }
                    else if key == "oauth" { node.insert(key.into(), Value::Bool(true)); }
                }
                Value::Object(node)
            };
            let digest = super::paths::hash(&serde_json::to_vec(&snapshot).map_err(|_| Error::InvalidFormat)?);
            let result = probe_node(&node, name, server.transport, digest, target)?;
            self.validate_path(parent)?; self.validate(false)?;
            Ok(result)
        }
        #[cfg(not(unix))] { let _ = (target, scope, name); Err(Error::Unsupported) }
    }
}

#[cfg(test)]
mod probe_boundary_tests {
    use super::*;
    #[test]
    fn projection_never_returns_literal_credentials() {
        let config = probe_node(&json!({"command":"server", "env":{"TOKEN":"secret-body"}, "headers":{"Authorization":"Bearer secret-body"}}), "server", Transport::Stdio, "digest".into(), Target::Claude).unwrap();
        let rendered = serde_json::to_string(&config.projection).unwrap();
        assert!(!rendered.contains("secret-body")); assert!(rendered.contains("native env:TOKEN"));
    }
    #[test]
    fn unsafe_existing_arguments_are_redacted() {
        let config = probe_node(&json!({"command":"server", "args":["--token", "secret-body"]}), "server", Transport::Stdio, "digest".into(), Target::Codex).unwrap();
        assert!(config.projection.redacted); assert!(config.projection.args.is_empty());
    }
    #[test]
    fn only_claude_env_literals_are_native_environment_references() {
        let node = json!({"command":"server", "env":{"TOKEN":"${NATIVE_TOKEN}"}});
        let codex = probe_node(&node, "server", Transport::Stdio, "digest".into(), Target::Codex).unwrap();
        let claude = probe_node(&node, "server", Transport::Stdio, "digest".into(), Target::Claude).unwrap();
        assert_eq!(codex.projection.credential_references, vec!["native env:TOKEN"]);
        assert_eq!(claude.projection.credential_references, vec!["env:NATIVE_TOKEN"]);
    }
    #[test]
    fn probes_reject_mixed_transport_fields_and_ambiguous_credentials() {
        let fields = ServerPatch { transport: Field::Set(Transport::Http), url: Field::Set("https://example.com/mcp".into()), forwarded_env: Field::Set(vec!["TOKEN".into()]), ..Default::default() };
        assert!(matches!(probe_draft(Target::Codex, &Scope::OsUser, "server".into(), fields), Err(Error::InvalidFormat)));
        assert!(matches!(probe_node(&json!({"url":"https://example.com/mcp", "http_headers":{"Authorization":"native"}, "env_http_headers":{"authorization":"TOKEN"}}), "server", Transport::Http, "digest".into(), Target::Codex), Err(Error::InvalidFormat)));
        assert!(matches!(probe_node(&json!({"url":"https://example.com/mcp", "http_headers":{"Authorization":"native"}, "bearer_token_env_var":"TOKEN"}), "server", Transport::Http, "digest".into(), Target::Codex), Err(Error::InvalidFormat)));
    }
}

/// Merge reference edits against the locked preimage. Literal native
/// credentials and unrelated native keys are never replaced.
pub(super) fn apply(text: &str, target: Target, scope: &Scope, project: Option<&str>, change: &Change, patches: &[Patch]) -> Result<String, Error> {
    if target != Target::Claude { return config::apply(text, target, patches); }
    let Change::Upsert { name, fields } = change else { return config::apply(text, target, patches); };
    if matches!(fields.forwarded_env, Field::Keep) && matches!(fields.header_env, Field::Keep) { return config::apply(text, target, patches); }
    let document = if text.trim().is_empty() { json!({}) } else { config::json_read(text)? };
    let mut server_path = base(scope, project)?; server_path.push(name.clone());
    let mut patched = patches.to_vec();
    for (key, changed) in [("env", !matches!(fields.forwarded_env, Field::Keep)), ("headers", !matches!(fields.header_env, Field::Keep))] {
        if !changed { continue; }
        let mut node = &document;
        let mut path = server_path.clone(); path.push(key.into());
        let empty = Value::Null;
        for part in &path { node = node.get(part).unwrap_or(&empty); }
        if key == "headers" {
            if node.is_null() { continue; }
            let values = node.as_object().ok_or(Error::InvalidFormat)?;
            if let Field::Set(headers) = &fields.header_env {
                for (name, _) in headers {
                    if values.iter().any(|(existing, value)| existing.eq_ignore_ascii_case(name) && !value.as_str().is_some_and(|value| reference(value).is_some())) { return Err(Error::Conflict); }
                }
            }
            // Header references are individual field patches, not a replacement
            // map: leave every unlisted literal and reference untouched.
            continue;
        }
        let mut retained = match node { Value::Null => serde_json::Map::new(), Value::Object(values) => values.iter().filter(|(_,v)| !v.as_str().is_some_and(|v| reference(v).is_some())).map(|(k,v)| (k.clone(), v.clone())).collect(), _ => return Err(Error::InvalidFormat) };
        let (_, replacement) = patched.iter_mut().find(|(keys,_)| *keys == path).ok_or(Error::InvalidFormat)?;
        if let Some(Value::Object(references)) = replacement {
            for (name, value) in references.iter() {
                if retained.contains_key(name) { return Err(Error::Conflict); }
                retained.insert(name.clone(), value.clone());
            }
        }
        *replacement = if retained.is_empty() { None } else { Some(Value::Object(retained)) };
    }
    config::apply(text, target, &patched)
}

#[cfg(test)]
mod forwarding_tests {
    use super::*;
    #[test]
    fn claude_forwarding_preserves_literal_credentials_and_unknown_fields() {
        let original = r#"{"future":{"x":1},"mcpServers":{"test":{"command":"server","future":{"flag":true},"env":{"LITERAL":"host-secret","OLD":"${OLD}"}}}}"#;
        let change = Change::Upsert { name: "test".into(), fields: ServerPatch { forwarded_env: Field::Set(vec!["NEW".into()]), ..Default::default() } };
        let patches = patches(Target::Claude, &Scope::OsUser, None, change.clone()).unwrap();
        let output = apply(original, Target::Claude, &Scope::OsUser, None, &change, &patches).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["mcpServers"]["test"]["env"], json!({"LITERAL":"host-secret", "NEW":"${NEW}"}));
        assert_eq!(value["future"]["x"], 1); assert_eq!(value["mcpServers"]["test"]["future"]["flag"], true);
        let change = Change::Upsert { name: "test".into(), fields: ServerPatch { forwarded_env: Field::Remove, ..Default::default() } };
        let patches = super::patches(Target::Claude, &Scope::OsUser, None, change.clone()).unwrap();
        let output = apply(&output, Target::Claude, &Scope::OsUser, None, &change, &patches).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["mcpServers"]["test"]["env"], json!({"LITERAL":"host-secret"}));
    }
    #[test]
    fn claude_forwarding_cannot_overwrite_native_literal() {
        let original = r#"{"mcpServers":{"test":{"command":"server","env":{"TOKEN":"native-secret"}}}}"#;
        let change = Change::Upsert { name: "test".into(), fields: ServerPatch { forwarded_env: Field::Set(vec!["TOKEN".into()]), ..Default::default() } };
        let patches = patches(Target::Claude, &Scope::OsUser, None, change.clone()).unwrap();
        assert_eq!(apply(original, Target::Claude, &Scope::OsUser, None, &change, &patches).unwrap_err(), Error::Conflict);
    }
    #[test]
    fn claude_header_references_preserve_literal_credentials_and_reject_case_aliases() {
        let original = r#"{"mcpServers":{"test":{"command":"server","headers":{"Authorization":"Bearer native-secret","Old":"${OLD}"}}}}"#;
        let change = Change::Upsert { name: "test".into(), fields: ServerPatch { header_env: Field::Set(vec![("New".into(), "NEW".into())]), ..Default::default() } };
        let output = apply(original, Target::Claude, &Scope::OsUser, None, &change, &patches(Target::Claude, &Scope::OsUser, None, change.clone()).unwrap()).unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["mcpServers"]["test"]["headers"], json!({"Authorization":"Bearer native-secret", "Old":"${OLD}", "New":"${NEW}"}));
        let change = Change::Upsert { name: "test".into(), fields: ServerPatch { header_env: Field::Set(vec![("authorization".into(), "TOKEN".into())]), ..Default::default() } };
        assert_eq!(apply(original, Target::Claude, &Scope::OsUser, None, &change, &patches(Target::Claude, &Scope::OsUser, None, change.clone()).unwrap()).unwrap_err(), Error::Conflict);
    }
}
