use super::*;
pub use dgos_db::{ChatRun, ChatRunStep, ChatSummary, ChatUsageSummary};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum ChatSkillMode { #[default] Manual, OnDemand }
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectChatContextInput {
    pub session_id:String, pub content:String,
    #[serde(default)] pub asset_ids:Vec<String>,
    #[serde(default)] pub tools:Vec<dgos_types::AiToolSelection>,
    #[serde(default)] pub skill_mode:ChatSkillMode,
    #[serde(default)] pub authorized_skill_refs:Vec<dgos_types::AiSkillRef>,
    #[serde(default)] pub foundation:dgos_types::AiFoundationOptions,
    #[serde(default)] pub web_access:crate::web::WebAccess,
}
#[derive(Clone, Debug, Serialize)]
pub struct ChatContextBudget {
    pub model_id:String, pub model_window_tokens:Option<i32>, pub host_text_limit_bytes:usize,
    pub reserved_output_tokens:u32, pub estimated_input_tokens:usize,
    pub text_bytes:usize, pub image_reserve_tokens:usize, pub skill_bytes:usize,
    pub tool_schema_bytes:usize, pub history_bytes:usize, pub dropped_turns:u32,
    pub summary:Option<ChatSummary>, pub can_send:bool, pub rejection:Option<String>,
    pub can_summarize:bool, pub summary_unavailable_reason:Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummarizeChatContextInput {
    pub session_id:String, pub client_request_id:String, pub confirmed:bool,
    #[serde(default)] pub accept_destination:Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListChatRunsInput { pub session_id:String }
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetChatRunInput { pub task_id:String }
#[derive(Clone, Debug, Serialize)]
pub struct ChatRunPage { pub items:Vec<ChatRun> }
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinueChatRunInput {
    pub source_task_id:String, pub client_request_id:String, pub intent:String,
    #[serde(default)] pub accept_destination:Option<String>,
    #[serde(default)] pub tools:Vec<dgos_types::AiToolSelection>,
    #[serde(default)] pub skill_mode:ChatSkillMode,
    #[serde(default)] pub authorized_skill_refs:Vec<dgos_types::AiSkillRef>,
    #[serde(default)] pub foundation:dgos_types::AiFoundationOptions,
    #[serde(default)] pub web_access:crate::web::WebAccess,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSkillCatalogInput {
    #[serde(default)] pub query:String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ChatSkillCatalog {
    pub items:Vec<dgos_skills::SkillCatalogEntry>,
    pub recommendations:Vec<dgos_skills::SkillRecommendation>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveChatRunStepInput {
    pub task_id:String, pub step_id:String, pub confirmed:bool,
    pub outcome:String, pub note:String,
}
