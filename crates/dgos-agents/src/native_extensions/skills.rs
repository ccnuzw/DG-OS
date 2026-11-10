//! Loose skill discovery and native visibility formatting. No skill content,
//! dynamic command, script, plugin hook or dependency program is executed.
use super::{config, Error, HostContext, Owner, Scope, Target};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VisibilityChange {
    CodexPath { selector: String, visible: bool },
    ClaudeName { name: String, state: RuleState },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleState { On, NameOnly, UserInvocableOnly, Off, Inherit }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule { pub name: String, pub state: RuleState }
#[derive(Clone, Debug, Serialize)]
pub struct Rules { pub rules: Vec<Rule>, pub coverage: &'static str }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleEvidence {
    pub evidence_version: u8, pub name: String, pub state: RuleState,
    pub previous: Option<RuleState>, pub scope: Scope,
    pub current_and_future: bool, pub plugins_excluded: bool,
    pub higher_precedence: String, pub config_change_hook_possible: bool,
}
pub(super) fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.len() > 512 || name.trim() != name || name.chars().any(char::is_control) { return Err(Error::InvalidFormat); }
    Ok(())
}
pub(super) fn read_rules(text: &str) -> Result<Vec<Rule>, Error> {
    let value = config::json_read(if text.is_empty() { "{}" } else { text })?;
    let Some(overrides) = value.get("skillOverrides") else { return Ok(Vec::new()); };
    let overrides = overrides.as_object().ok_or(Error::InvalidFormat)?;
    overrides.iter().map(|(name, value)| {
        validate_name(name)?;
        let state: RuleState = serde_json::from_value(value.clone()).map_err(|_| Error::InvalidFormat)?;
        if state == RuleState::Inherit { return Err(Error::InvalidFormat); }
        Ok(Rule { name: name.clone(), state })
    }).collect()
}
pub(super) fn rule_evidence(text: &str, scope: Scope, change: &VisibilityChange) -> Result<Option<RuleEvidence>, Error> {
    let VisibilityChange::ClaudeName { name, state } = change else { return Ok(None); };
    validate_name(name)?;
    let previous = read_rules(text)?.into_iter().find(|rule| rule.name == *name).map(|rule| rule.state);
    Ok(Some(RuleEvidence { evidence_version: 1, name: name.clone(), state: *state, previous, scope,
        current_and_future: true, plugins_excluded: true, higher_precedence: "not_evaluated".into(), config_change_hook_possible: true }))
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
    let VisibilityChange::CodexPath { selector, visible } = change else { return Err(Error::InvalidFormat); };
    if !std::path::Path::new(selector).is_absolute() || selector.len() > 4096 || !selector.ends_with("/SKILL.md") || selector.contains(['\0', '\n', '\r']) { return Err(Error::InvalidFormat); }
    let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
    if doc.get("skills").is_none() { doc["skills"] = toml_edit::Item::Table(toml_edit::Table::new()); }
    let skills = doc.get_mut("skills").and_then(toml_edit::Item::as_table_like_mut).ok_or(Error::InvalidFormat)?;
    if !skills.contains_key("config") { skills.insert("config", toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new())); }
    let configs = skills.get_mut("config").and_then(toml_edit::Item::as_array_of_tables_mut).ok_or(Error::InvalidFormat)?;
    let matching: Vec<usize> = configs.iter().enumerate().filter(|(_,t)| t.get("path").and_then(toml_edit::Item::as_str) == Some(selector.as_str())).map(|(i,_)| i).collect();
    if matching.len() > 1 { return Err(Error::Conflict); }
    if let Some(index) = matching.first() {
        let table = configs.get_mut(*index).ok_or(Error::InvalidFormat)?;
        if table.contains_key("name") { return Err(Error::InvalidFormat); }
        let mut enabled = toml_edit::Value::from(*visible);
        if let Some(old) = table.get("enabled").and_then(toml_edit::Item::as_value) { *enabled.decor_mut() = old.decor().clone(); }
        table.insert("enabled", toml_edit::Item::Value(enabled));
    } else {
        let mut table = toml_edit::Table::new();
        table.insert("path", toml_edit::value(selector.as_str()));
        table.insert("enabled", toml_edit::value(*visible));
        configs.push(table);
    }
    let output = doc.to_string();
    if output.len() > config::CONFIG_LIMIT { return Err(Error::Limit); }
    output.parse::<toml_edit::DocumentMut>().map_err(|_| Error::InvalidFormat)?;
    Ok(output)
}
pub(super) fn edit_claude_visibility(text: &str, change: &VisibilityChange) -> Result<String, Error> {
    let VisibilityChange::ClaudeName { name, state } = change else { return Err(Error::InvalidFormat); };
    validate_name(name)?;
    read_rules(text)?;
    let value = if *state == RuleState::Inherit { None } else { Some(serde_json::to_value(state).map_err(|_| Error::InvalidFormat)?) };
    config::apply(text, Target::Claude, &[(vec!["skillOverrides".into(), name.clone()], value)])
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
    let root_owner = super::resources::owner(&root, "")?;
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
        let owner = if root_owner == Owner::Loose { super::resources::owner(&child, &name)? } else { root_owner };
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
