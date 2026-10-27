use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUsageLog {
    pub id: String,
    pub skill_id: String,
    pub user_id: String,
    pub input_params: JsonValue,
    pub output_result: Option<JsonValue>,
    pub success: bool,
    pub error_message: Option<String>,
    pub latency_ms: i32,
    pub created_at: i64,
}
