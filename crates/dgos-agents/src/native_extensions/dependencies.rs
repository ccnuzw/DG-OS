//! Static declarations and host-environment lookup. No subprocess or environment expansion.
use super::Error;
use serde::Serialize;
use std::path::PathBuf;
#[derive(Clone, Debug, Default, Serialize)]
pub struct Declarations { pub declared: bool, pub tools: Vec<Tool>, pub packages: Vec<String>, pub required_tools: Vec<String> }
#[derive(Clone, Debug, Serialize)]
pub struct Tool { pub program: String, pub version_requirement: Option<String> }
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub enum Availability { FoundUnprobed, Missing, Unreadable, Unknown }
#[derive(Clone, Debug, Serialize)]
pub struct LocatedTool { pub declaration: Tool, pub availability: Availability, pub candidate: Option<PathBuf>, pub executable: bool }
#[derive(Clone, Debug, Serialize)]
pub struct Report { pub declarations: Declarations, pub tools: Vec<LocatedTool>, pub unknown: bool }
/// Host-only PATH snapshot: never Deserialize or populated from arbitrary renderer environment.
pub struct Environment { paths: Vec<PathBuf> }
fn text(v: &str) -> Result<String, Error> { if v.len() > 256 || v.chars().any(char::is_control) { Err(Error::InvalidFormat) } else { Ok(v.to_owned()) } }
fn program(v: &str) -> Result<String, Error> { if v.is_empty() || v.len() > 128 || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) { Err(Error::InvalidFormat) } else { Ok(v.into()) } }
fn list(value: Option<&serde_yaml::Value>) -> Result<Vec<String>, Error> {
    let Some(value) = value else { return Ok(Vec::new()); };
    let sequence = value.as_sequence().ok_or(Error::InvalidFormat)?;
    if sequence.len() > 128 { return Err(Error::Limit); }
    sequence.iter().map(|v| text(v.as_str().ok_or(Error::InvalidFormat)?)).collect()
}
impl Declarations {
    pub(super) fn parse(value: &serde_yaml::Value) -> Result<Self, Error> {
        let dependencies = value.get("dependencies");
        let mut result = Self { declared: dependencies.is_some() || value.get("required_tools").is_some(), ..Default::default() };
        if let Some(deps) = dependencies {
            let map = deps.as_mapping().ok_or(Error::InvalidFormat)?;
            for (key, value) in map { match key.as_str() {
                Some("python" | "node") => { let key = key.as_str().ok_or(Error::InvalidFormat)?; result.tools.push(Tool { program: if key == "python" { "python3" } else { "node" }.into(), version_requirement: Some(text(value.as_str().ok_or(Error::InvalidFormat)?)?) }); },
                Some("tools") => for tool in list(Some(value))? { result.tools.push(Tool { program: program(&tool)?, version_requirement: None }); },
                Some("packages") => result.packages = list(Some(value))?,
                _ => {},
            } }
        }
        result.required_tools = list(value.get("required_tools"))?;
        Ok(result)
    }
}
impl Environment {
    pub fn from_host(paths: Vec<PathBuf>) -> Result<Self, Error> {
        if paths.len() > 128 || paths.iter().any(|p| !p.is_absolute()) { return Err(Error::UnsafePath); }
        Ok(Self { paths })
    }
    pub fn locate(&self, declarations: &Declarations) -> Report {
        let tools = declarations.tools.iter().map(|tool| {
            let mut availability = Availability::Missing; let mut candidate = None; let mut executable = false;
            for root in &self.paths {
                let path = root.join(&tool.program);
                match std::fs::metadata(&path) {
                    Ok(meta) if meta.is_file() => {
                        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; executable = meta.permissions().mode() & 0o111 != 0; }
                        if executable { availability = Availability::FoundUnprobed; candidate = Some(path); break; }
                        availability = Availability::Unreadable;
                    },
                    Ok(_) => {},
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                    Err(_) => availability = Availability::Unreadable,
                }
            }
            LocatedTool { declaration: tool.clone(), availability, candidate, executable }
        }).collect();
        // Location is never proof of a declared version/package/API requirement.
        Report { declarations: declarations.clone(), tools, unknown: !declarations.declared || !declarations.tools.is_empty() || !declarations.packages.is_empty() || !declarations.required_tools.is_empty() }
    }
}
