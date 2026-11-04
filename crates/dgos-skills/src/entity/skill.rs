use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: String,
    pub author: Option<String>,
    pub install_path: String,
    pub call_type: CallType,
    pub enabled: bool,
    pub config: Option<JsonValue>,
    pub manifest: SkillManifest,
    pub installed_at: i64,
    pub updated_at: i64,
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CallType {
    Http,
    Cli,
    Content,
}

impl CallType {
    pub fn as_str(&self) -> &str {
        match self {
            CallType::Http => "http",
            CallType::Cli => "cli",
            CallType::Content => "content",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("http") {
            Some(CallType::Http)
        } else if s.eq_ignore_ascii_case("cli") {
            Some(CallType::Cli)
        } else if s.eq_ignore_ascii_case("content") {
            Some(CallType::Content)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub category: String,
    pub tags: Vec<String>,
    pub call_type: CallType,
    pub dependencies: DependencyConfig,
    pub config_schema: Option<JsonValue>,
    pub http: Option<HttpConfig>,
    pub cli: Option<CliConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyConfig {
    pub python: Option<String>,
    pub node: Option<String>,
    pub packages: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    pub port: u16,
    pub start_command: String,
    pub health_check: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUpdate {
    pub skill_id: String,
    pub current_version: String,
    pub latest_version: String,
    pub changelog: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub satisfied: bool,
    pub missing: Vec<String>,
}
