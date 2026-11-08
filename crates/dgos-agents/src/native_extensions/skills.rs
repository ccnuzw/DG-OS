//! Loose skill discovery and native visibility formatting. No skill content,
//! dynamic command, script, plugin hook or dependency program is executed.
use super::{config, Error, HostContext, Owner, Scope, Target};
use serde::Serialize;

#[derive(Clone)]
pub struct VisibilityChange {
    /// Codex: exact absolute SKILL.md path derived by the trusted host.
    /// Claude: exact native command name; it may affect more than one instance.
    pub selector: String,
    pub visible: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Instance {
    pub directory_name: String,
    pub owner: Owner,
    pub has_skill_entry: bool,
    pub has_scripts: bool,
    pub readable: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Discovery { pub instances: Vec<Instance>, pub incomplete: bool }

pub(super) fn edit_codex_visibility(text: &str, change: &VisibilityChange) -> Result<String, Error> {
    if !std::path::Path::new(&change.selector).is_absolute() || change.selector.len() > 4096 || !change.selector.ends_with("/SKILL.md") || change.selector.contains(['\0', '\n', '\r']) { return Err(Error::InvalidFormat); }
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
    if doc.get("skills").is_none() { doc["skills"] = toml_edit::Item::Table(toml_edit::Table::new()); }
    let skills = doc.get_mut("skills").and_then(toml_edit::Item::as_table_like_mut).ok_or(Error::InvalidFormat)?;
    if !skills.contains_key("config") { skills.insert("config", toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new())); }
    let configs = skills.get_mut("config").and_then(toml_edit::Item::as_array_of_tables_mut).ok_or(Error::InvalidFormat)?;
    let matching: Vec<usize> = configs.iter().enumerate().filter(|(_,t)| t.get("path").and_then(toml_edit::Item::as_str) == Some(change.selector.as_str())).map(|(i,_)| i).collect();
    if matching.len() > 1 { return Err(Error::Conflict); }
    if let Some(index) = matching.first() {
        let table = configs.get_mut(*index).ok_or(Error::InvalidFormat)?;
        if table.contains_key("name") { return Err(Error::InvalidFormat); }
        let mut enabled = toml_edit::Value::from(change.visible);
        if let Some(old) = table.get("enabled").and_then(toml_edit::Item::as_value) { *enabled.decor_mut() = old.decor().clone(); }
        table.insert("enabled", toml_edit::Item::Value(enabled));
    } else {
        let mut table = toml_edit::Table::new();
        table.insert("path", toml_edit::value(change.selector.as_str()));
        table.insert("enabled", toml_edit::value(change.visible));
        configs.push(table);
    }
    let output = doc.to_string();
    if output.len() > config::CONFIG_LIMIT { return Err(Error::Limit); }
    output.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
    Ok(output)
}
pub(super) fn edit_claude_visibility(text: &str, change: &VisibilityChange) -> Result<String, Error> {
    if change.selector.is_empty() || change.selector.len() > 256 || change.selector.contains(['\0', '\n', '\r', '/', '\\']) { return Err(Error::InvalidFormat); }
    // Plugin visibility belongs to the plugin manager, not skillOverrides.
    if change.selector.contains(':') { return Err(Error::Unsupported); }
    config::apply(text, Target::Claude, &[(vec!["skillOverrides".into(), change.selector.clone()], Some(serde_json::Value::String(if change.visible { "on" } else { "off" }.into())))])
}
#[cfg(unix)]
pub(super) fn discover(context: &HostContext, target: Target, scope: Scope) -> Result<Discovery, Error> {
    use super::paths::Directory;
    let path = match (target, scope) {
        (Target::Codex, Scope::OsUser) => context.roots.os_home.join(".agents/skills"),
        (Target::Claude, Scope::OsUser) => context.roots.claude_home.join("skills"),
        (Target::Codex, Scope::ProjectShared) => context.roots.project.as_ref().ok_or(Error::AccessDenied)?.join(".agents/skills"),
        (Target::Claude, Scope::ProjectShared) => context.roots.project.as_ref().ok_or(Error::AccessDenied)?.join(".claude/skills"),
        (_, Scope::ProjectLocal) => return Err(Error::Unsupported),
    };
    let root = match Directory::open(&path) {
        Ok(root) => root,
        Err(Error::NotFound) => return Ok(Discovery { instances: Vec::new(), incomplete: true }),
        Err(error) => return Err(error),
    };
    let (names, mut incomplete) = root.names(2048)?;
    let mut instances = Vec::new();
    for name in names {
        if name.starts_with('.') { continue; }
        let kind = root.kind(&name)?;
        if kind == "symlink" {
            instances.push(Instance { directory_name: name, owner: Owner::Symlink, has_skill_entry: false, has_scripts: false, readable: false }); continue;
        }
        if kind != "directory" { continue; }
        let child = match root.child(std::ffi::OsStr::new(&name), false) {
            Ok(dir) => dir,
            Err(_) => { incomplete = true; instances.push(Instance { directory_name: name, owner: Owner::Unknown, has_skill_entry: false, has_scripts: false, readable: false }); continue; }
        };
        let owner = if target == Target::Claude && name == "synced" { Owner::Synced }
            else if child.kind(".claude-plugin")? == "directory" { Owner::Plugin } else { Owner::Loose };
        let has_scripts = child.kind("scripts")? == "directory";
        let (has_skill_entry, readable) = if child.kind("SKILL.md")? == "file" {
            match child.read_bounded("SKILL.md", 256 * 1024) {
                Ok(Some(_)) => (true, true),
                _ => { incomplete = true; (true, false) }
            }
        } else { (false, false) };
        // Unknown frontmatter and bodies remain on disk, never in projection.
        instances.push(Instance { directory_name: name, owner, has_skill_entry, has_scripts, readable });
    }
    Ok(Discovery { instances, incomplete })
}
