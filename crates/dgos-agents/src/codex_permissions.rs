//! Codex rust-v0.167.0 permission overlays. Approval echoes the native request;
//! it never accepts edited paths, broader scopes, or a command-approval substitute.
use serde_json::Value;

fn keys(value: &Value, allowed: &[&str]) -> bool {
    value.as_object().is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
}
fn optional(value: &Value, key: &str, check: impl Fn(&Value) -> bool) -> bool {
    value.get(key).is_none_or(|field| field.is_null() || check(field))
}
fn text(value: &Value) -> bool {
    value.as_str().is_some_and(|value| !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control))
}
fn paths(value: &Value) -> bool { value.as_array().is_some_and(|values| values.len() <= 256 && values.iter().all(text)) }
fn path(value: &Value) -> bool {
    match value["type"].as_str() {
        Some("path") => keys(value, &["type", "path"]) && text(&value["path"]),
        Some("glob_pattern") => keys(value, &["type", "pattern"]) && text(&value["pattern"]),
        Some("special") if keys(value, &["type", "value"]) => {
            let special = &value["value"];
            match special["kind"].as_str() {
                Some("root" | "minimal" | "tmpdir" | "slash_tmp") => keys(special, &["kind"]),
                Some("project_roots" | "current_working_directory") => keys(special, &["kind", "subpath"]) && optional(special, "subpath", text),
                // Unknown native variants are not a grantable path.
                _ => false,
            }
        }
        _ => false,
    }
}
fn profile(value: &Value) -> bool {
    keys(value, &["network", "fileSystem"])
        && optional(value, "network", |network| keys(network, &["enabled"]) && optional(network, "enabled", Value::is_boolean))
        && optional(value, "fileSystem", |fs| {
            keys(fs, &["read", "write", "entries", "globScanMaxDepth"])
                && optional(fs, "read", paths) && optional(fs, "write", paths)
                && optional(fs, "globScanMaxDepth", |depth| depth.as_u64().is_some_and(|depth| depth > 0))
                && optional(fs, "entries", |entries| entries.as_array().is_some_and(|entries| entries.len() <= 256 && entries.iter().all(|entry| {
                    keys(entry, &["path", "access"]) && path(&entry["path"]) && matches!(entry["access"].as_str(), Some("read" | "write" | "deny"))
                })))
        })
}
pub(super) fn valid_request(params: &Value) -> bool {
    keys(params, &["threadId", "turnId", "itemId", "cwd", "startedAtMs", "environmentId", "reason", "permissions"])
        && ["threadId", "turnId", "itemId", "cwd"].iter().all(|key| text(&params[*key]))
        && params["startedAtMs"].is_i64()
        && optional(params, "environmentId", text)
        && optional(params, "reason", |value| value.as_str().is_some_and(|text| text.len() <= 16 * 1024))
        && profile(&params["permissions"])
}
pub(super) fn valid_response(params: &Value, response: &Value) -> bool {
    if !valid_request(params) || !keys(response, &["permissions", "scope"]) { return false; }
    let Some(scope) = response["scope"].as_str() else { return false; };
    let permissions = &response["permissions"];
    // Empty overlay denies this request. A session-wide empty grant is misleading.
    (scope == "turn" && permissions.as_object().is_some_and(|object| object.is_empty()))
        || (matches!(scope, "turn" | "session") && permissions == &params["permissions"])
}

pub(super) fn turn_policy(mode: crate::session::PermissionMode, planning: bool, sandbox: Option<&Value>) -> Result<Value, String> {
    use crate::session::PermissionMode;
    if planning || mode == PermissionMode::ReadOnly {
        return Ok(serde_json::json!({"approvalPolicy":"never","approvalsReviewer":"user","sandboxPolicy":{"type":"readOnly","networkAccess":false}}));
    }
    if mode == PermissionMode::AcceptEdits { return Err("unsupported_permission_mode".into()); }
    // Full auto removes approval prompts only inside the original workspace sandbox.
    // It never substitutes dangerFullAccess or broadens writable roots/network.
    Ok(serde_json::json!({"approvalPolicy":if mode == PermissionMode::FullAuto { "never" } else { "on-request" },"approvalsReviewer":"user","sandboxPolicy":sandbox.ok_or("unsupported_permission_mode")?}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request() -> Value { json!({"threadId":"thread","turnId":"turn","itemId":"item","cwd":"/fixture","startedAtMs":1,"permissions":{"network":{"enabled":true},"fileSystem":{"read":["/fixture/input"],"write":null}}}) }
    #[test]
    fn permissions_cannot_be_broadened_or_replaced_with_command_decisions() {
        let request = request();
        for scope in ["turn", "session"] { assert!(valid_response(&request, &json!({"permissions":request["permissions"],"scope":scope}))); }
        assert!(valid_response(&request, &json!({"permissions":{},"scope":"turn"})));
        for response in [json!({"permissions":{},"scope":"session"}), json!({"decision":"accept"}), json!({"permissions":request["permissions"],"scope":"permanent"}), json!({"permissions":request["permissions"],"scope":"turn","strictAutoReview":false}), json!({"permissions":{"fileSystem":{"write":["/"]}},"scope":"turn"})] {
            assert!(!valid_response(&request, &response));
        }
    }
    #[test]
    fn unsupported_permission_shapes_fail_closed() {
        let mut request = request();
        request["futureAuthorization"] = json!(true);
        assert!(!valid_request(&request));
        request.as_object_mut().unwrap().remove("futureAuthorization");
        request["permissions"]["fileSystem"] = json!({"entries":[{"path":{"type":"special","value":{"kind":"project_roots","subpath":null}},"access":"read"}],"globScanMaxDepth":2});
        assert!(valid_request(&request));
        request["permissions"]["fileSystem"]["entries"][0]["path"]["value"]["kind"] = json!("unknown");
        assert!(!valid_request(&request));
        request["permissions"] = json!({"network":{"enabled":true,"futureExpansion":true}});
        assert!(!valid_request(&request));
    }
    #[test]
    fn full_auto_keeps_workspace_scope_and_planning_always_tightens_it() {
        use crate::session::PermissionMode;
        let sandbox = json!({"type":"workspaceWrite","networkAccess":false,"writableRoots":["/fixture"]});
        let auto = turn_policy(PermissionMode::FullAuto, false, Some(&sandbox)).unwrap();
        assert_eq!(auto["approvalPolicy"], "never");
        assert_eq!(auto["sandboxPolicy"], sandbox);
        let plan = turn_policy(PermissionMode::FullAuto, true, Some(&sandbox)).unwrap();
        assert_eq!(plan["sandboxPolicy"]["type"], "readOnly");
        assert_eq!(plan["sandboxPolicy"]["networkAccess"], false);
        assert_eq!(turn_policy(PermissionMode::FullAuto, false, Some(&sandbox)).unwrap(), auto);
    }
}
