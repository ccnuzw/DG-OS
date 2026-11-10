use dgos_agents::native_extensions as core;
use serde::{Deserialize, Serialize};
pub use core::{Receipt, ResourceInstance, Scope, Target};
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Selection { pub target: Target, pub scope: Scope, pub project_id: Option<String> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverSkills { pub target: Target, pub scope: Scope, pub project_id: Option<String>, pub query: String }
impl DiscoverSkills { pub fn selection(&self) -> Selection { Selection { target: self.target, scope: self.scope.clone(), project_id: self.project_id.clone() } } }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetSkill { pub target: Target, pub scope: Scope, pub project_id: Option<String>, pub slug: String }
impl GetSkill { pub fn selection(&self) -> Selection { Selection { target: self.target, scope: self.scope.clone(), project_id: self.project_id.clone() } } }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source { Zip { bytes: Vec<u8>, subpath: Option<String> }, Github { input: String } }
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillAction { Install, Update, Remove }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewSkill { pub target: Target, pub scope: Scope, pub project_id: Option<String>, pub action: SkillAction, pub slug: String, pub expected: Option<ResourceInstance>, pub source: Option<Source> }
impl PreviewSkill { pub fn selection(&self) -> Selection { Selection { target: self.target, scope: self.scope.clone(), project_id: self.project_id.clone() } } }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visibility { pub target: Target, pub scope: Scope, pub project_id: Option<String>, pub intent: VisibilityIntent }
impl Visibility { pub fn selection(&self) -> Selection { Selection { target: self.target, scope: self.scope.clone(), project_id: self.project_id.clone() } } }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VisibilityIntent {
    CodexPath { instance: ResourceInstance, visible: bool },
    ClaudeName { name: String, state: core::skills::RuleState },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation { pub operation_id: String }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Field<T> { Keep, Set { value: T }, Remove }
impl<T> Default for Field<T> { fn default() -> Self { Self::Keep } }
impl<T> Field<T> { fn map(self) -> core::mcp::Field<T> { match self { Self::Keep => core::mcp::Field::Keep, Self::Set { value } => core::mcp::Field::Set(value), Self::Remove => core::mcp::Field::Remove } } }
#[derive(Deserialize)]
pub enum Transport { Stdio, Http, Sse, WebSocket, Unknown }
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServerPatch { pub transport: Field<Transport>, pub program: Field<String>, pub args: Field<Vec<String>>, pub url: Field<String>, pub cwd: Field<String>, pub bearer_env: Field<String>, pub forwarded_env: Field<Vec<String>>, pub header_env: Field<Vec<(String, String)>> }
impl ServerPatch {
    fn map(self) -> core::mcp::ServerPatch {
        let transport = match self.transport { Field::Keep => core::mcp::Field::Keep, Field::Remove => core::mcp::Field::Remove, Field::Set { value } => core::mcp::Field::Set(match value { Transport::Stdio => core::mcp::Transport::Stdio, Transport::Http => core::mcp::Transport::Http, Transport::Sse => core::mcp::Transport::Sse, Transport::WebSocket => core::mcp::Transport::WebSocket, Transport::Unknown => core::mcp::Transport::Unknown }) };
        core::mcp::ServerPatch { transport, program: self.program.map(), args: self.args.map(), url: self.url.map(), cwd: self.cwd.map(), bearer_env: self.bearer_env.map(), forwarded_env: self.forwarded_env.map(), header_env: self.header_env.map() }
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpChange { Upsert { name: String, fields: ServerPatch }, Remove { name: String }, SetEnabled { name: String, enabled: bool } }
impl McpChange { pub fn map(self) -> core::mcp::Change { match self { Self::Upsert { name, fields } => core::mcp::Change::Upsert { name, fields: fields.map() }, Self::Remove { name } => core::mcp::Change::Remove { name }, Self::SetEnabled { name, enabled } => core::mcp::Change::SetEnabled { name, enabled } } } }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewMcp { pub target: Target, pub scope: Scope, pub project_id: Option<String>, pub change: McpChange }
impl PreviewMcp { pub fn selection(&self) -> Selection { Selection { target: self.target, scope: self.scope.clone(), project_id: self.project_id.clone() } } }
#[derive(Serialize)]
pub struct SkillPreview { pub receipt: Receipt, pub target: Target, pub scope: Scope, pub project_id: Option<String>, pub slug: String, pub before_hash: Option<String>, pub after_hash: Option<String>, pub source: Option<core::ResourceSource>, pub resources: Vec<core::package::ResourceFile>, pub metadata: Option<core::package::Metadata>, pub impact: Option<core::ResourceImpact>, pub expires_at: u64 }
#[derive(Serialize)]
pub struct VisibilityEffect { pub selection: Selection, pub instance: ResourceInstance, pub slug: String }
#[derive(Serialize)]
pub struct VisibilityPreview { pub preview: core::Preview, pub affected: Vec<VisibilityEffect>, pub coverage: String, pub observation_coverage: String, pub rule: Option<core::skills::RuleEvidence> }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListNativeSessions { pub project_id: String, pub domain: dgos_agents::native_session::SessionDomain }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveNativeSession { pub session_id: String, pub generation: String, pub domain: dgos_agents::native_session::SessionDomain }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSessionControl { pub session_id: String, pub generation: String, pub request_id: String, pub action: dgos_agents::native_session::NativeSessionAction }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectNativeSessionRequest { pub session_id: String, pub generation: String, pub request_id: String, pub domain: dgos_agents::native_session::SessionDomain }
