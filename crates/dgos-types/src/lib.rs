// DG-OS Common Types
// Shared types across all modules

pub mod appearance;
pub mod error;
pub mod execution;
pub mod module_operation;
pub mod provider;

pub use appearance::{AppearancePatch, AppearancePreferences, Density, Theme};
pub use error::{DgosError, DgosResult};
pub use execution::{
    BuiltinCaller, ExecutionProvenance, ExecutionSource, HostExecutionContext, HostSession,
};
pub use module_operation::{
    ModuleCrashView, ModuleOperationAccepted, ModuleOperationErrorKind, ModuleOperationFailure,
    ModuleOperationKind, ModuleOperationOutcome, ModuleOperationState, ModuleOperationView,
    ModulePermissionView, ModuleRuntimeStatusState, ModuleRuntimeStatusView, ModuleSummaryView,
};
pub use provider::*;
