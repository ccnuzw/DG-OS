// DG-OS Common Types
// Shared types across all modules

pub mod ai;
pub use ai::*;
pub mod appearance;
pub mod error;
pub mod execution;
pub mod locale;
pub mod generation;
pub use generation::GenerationParameters;
pub mod model_catalog;
pub mod module_operation;
pub mod provider;

pub use appearance::{AppearanceRead, AppearanceSave, AppearanceSnapshot, AppearanceValues, FontSelection};
pub use error::{DgosError, DgosResult};
pub use execution::{
    BuiltinCaller, ExecutionProvenance, ExecutionSource, HostExecutionContext, HostSession,
    DEFAULT_USER_ID,
};
pub use locale::{
    resolve_locale, resolve_system_locale, LanguagePreference, LanguagePreferences, Locale,
};
pub use model_catalog::{
    build_model, capability_hints, classify_model, recompute_models, valid_model_id, Hint,
    KindSource, ModelCapabilities, ModelKind, ModelSource, MAX_MODEL_ID_CHARS, PROTOCOL_ANTHROPIC,
    PROTOCOL_OPENAI_COMPATIBLE,
};
pub use module_operation::{
    ModuleCrashView, ModuleOperationAccepted, ModuleOperationErrorKind, ModuleOperationFailure,
    ModuleOperationKind, ModuleOperationOutcome, ModuleOperationState, ModuleOperationView,
    ModulePermissionAuditPageView, ModulePermissionAuditView, ModulePermissionView,
    ModuleRuntimeStatusState, ModuleRuntimeStatusView, ModuleSummaryView,
    WorkspaceAppCatalog, WorkspaceAppEntry,
};
pub use provider::*;
