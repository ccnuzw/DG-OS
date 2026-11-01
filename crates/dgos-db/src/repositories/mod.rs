pub mod appearance_repository;
pub mod chat_repository;
pub mod lan_audit_repository;
pub mod mcp_credential_cleanup_repository;
pub mod model_repository;
pub mod module_crash_repository;
pub mod module_permission_repository;
pub mod module_storage_repository;
pub mod provider_repository;
pub mod usage_log_repository;
pub mod user_data_repository;
pub mod user_repository;

pub use appearance_repository::AppearanceRepository;
pub use chat_repository::{
    auto_title, like_pattern, result_is_unknown, snippet, BeginGeneration, Begun, ChatMessage,
    ChatRepository, ChatSession, ChatStoreError, ChatTarget, FinalizeGeneration, Finalized,
    History, HistoryRow, PinnedSkill, SearchHit, DEFAULT_TITLE,
};
pub use lan_audit_repository::{
    LanAuditEvent, LanAuditKind, LanAuditPage, LanAuditRecord, LanAuditRepository,
    LAN_AUDIT_MERGE_SECS, MAX_LAN_AUDIT_ROWS,
};
pub use mcp_credential_cleanup_repository::{
    stage_mcp_credential_cleanup, McpCredentialCleanup, McpCredentialCleanupRepository,
};
pub use model_repository::ModelRepository;
pub use module_crash_repository::{CrashRecord, ModuleCrashRepository, MAX_CRASHES_PER_MODULE};
pub use module_permission_repository::{
    ModulePermission, ModulePermissionAuditEntry, ModulePermissionAuditPage, ModulePermissionInput,
    ModulePermissionRepository, MAX_AUDITS_PER_USER,
};
pub use module_storage_repository::{
    ModuleStorageEntry, ModuleStorageRepository, MAX_STORAGE_KEY_CHARS, MAX_STORAGE_TOTAL_BYTES,
    MAX_STORAGE_VALUE_BYTES,
};
pub use provider_repository::ProviderRepository;
pub use usage_log_repository::UsageLogRepository;
pub use user_data_repository::{UserDataDomain, UserDataRepository, RETAINED_FOR_VAULT_RETRY};
pub use user_repository::{
    HostSettingsRepository, NewUser, UserRecord, UserRepository, LAST_SIGN_IN_USER,
};
