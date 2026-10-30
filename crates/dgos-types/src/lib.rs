// DG-OS Common Types
// Shared types across all modules

pub mod appearance;
pub mod error;
pub mod execution;
pub mod model_catalog;
pub mod module_operation;
pub mod provider;

pub use appearance::{AppearancePatch, AppearancePreferences, Density, Theme};
pub use error::{DgosError, DgosResult};
pub use execution::{
    BuiltinCaller, ExecutionProvenance, ExecutionSource, HostExecutionContext, HostSession,
};
pub use model_catalog::{
    build_model, capability_hints, classify_model, recompute_models, valid_model_id, Hint,
    KindSource, ModelCapabilities, ModelKind, ModelSource, MAX_MODEL_ID_CHARS, PROTOCOL_ANTHROPIC,
    PROTOCOL_OPENAI_COMPATIBLE,
};
pub use module_operation::{
    ModuleCrashView, ModuleOperationAccepted, ModuleOperationErrorKind, ModuleOperationFailure,
    ModuleOperationKind, ModuleOperationOutcome, ModuleOperationState, ModuleOperationView,
    ModulePermissionView, ModuleRuntimeStatusState, ModuleRuntimeStatusView, ModuleSummaryView,
};
pub use provider::*;
